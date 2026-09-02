---
type: Reference
title: mev CLI — validation commands
description: The four commands that check something and tell you what is wrong — learn-ai content, the Brain corpus, a single state.json, and cross-source drift.
doc_id: cli-validate
layer: [factory]
project: mev
status: active
keywords: [validation, diagnostics, OKF, state.json, conformance]
related: [cli-reference, okf-schema, architecture]
---

# mev CLI — validation commands

Part of the [CLI reference](../cli.md). These four commands **read and report**; none of them
writes to the corpus.

## What this page is for

You have a file, a repo, or the whole Brain, and you want to know whether it is well-formed before
something downstream chokes on it. These are the commands that answer that. Each prints diagnostics
with a **locator** — a stable code like `E_STATE_SCHEMA_MALFORMED_SCOPE` you can search for.

| Command | Answers |
|---|---|
| [`validate`](#validate---blog---lint-path) | Is the learn-ai content tree well-formed? |
| [`validate-brain`](#validate-brain---sync---graph---state---links---structure-path) | Is the whole Brain corpus well-formed? |
| [`validate-state`](#validate-state-path) | Is this one `state.json` well-formed? |
| [`conformance`](#conformance---check-name---json-path) | Do facts kept in two places still agree? |

## Quickstart

Run these in a **terminal**, from anywhere inside the brain (each walks up to find `brain.toml`):

```bash
# Is the corpus healthy? One flag per run — they do NOT compose.
bastion validate-brain --state
bastion validate-brain --links
bastion validate-brain --structure
bastion validate-brain --graph

# Check one file you just edited
mev validate-state core/mev/planning/state.json

# Check the learn-ai content tree
mev validate ../learn-ai/content/learn
```

**One flag per invocation.** `validate-brain`'s flag handling is an if/else-if chain — the first
flag wins and the rest are ignored, so a combined run silently checks less than you think.

**A piped exit code is the pipe's, not the command's.** `mev conformance | tail` reports success
while `mev conformance` exits 1. Redirect to a file, then read `$?`.

## Commands

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
| `E_LINT_DEAD_LOCAL_LINK` | Error | A markdown link `target` resolves to a path that does not exist. Absolute URLs (`http://`, `https://`, `mailto:`, protocol-relative `//`) and in-page anchors (`#...`) are skipped, never reported. A genuinely relative target (`./x.md`, `../assets/y.png`) resolves against the file's parent directory. A **site-absolute** target (a single leading `/`, e.g. `/en/blog/x`) is a Next.js route, not a filesystem path — see "Site-absolute route resolution" below; causes exit 1 |
| `E_LINT_DEAD_ASSET` | Error | An image reference `!target` resolves to a path that does not exist on disk; causes exit 1 |
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

See [brain-toml.md](../brain-toml.md) for the configuration format and [okf-schema.md](../okf-schema.md) for what is validated.

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
8. **`reference[]` validation** (`MV.ticket.reference-container-validation`) — checks the shape of every `reference[]` entry: `class` must be one of `trap`/`invariant`/`lesson`/`deliberate`; `scope` must set exactly one of `repo`/`tier`/`cross_repo` (`cross_repo: false` counts as set, same rule as `carryover[]` scope); `created`/`reviewed` must parse as a date; and a `slug` must not appear in both `reference[]` and `carryover[]` within the same file. `reference[]` entries are permanently-true material (traps, invariants, lessons, deliberate-choice markers) with no clock by design (D72 §5) — they are checked for shape only, never for staleness, and are never evaluated by `check_carryover_staleness` or emitted onto any triage surface (see [Carryover triage lanes](state.md#carryover-triage-lanes), [`attention-queue`](#attention-queue---out-path-path), and `mev carryover` below — none of them read `reference[]`).
9. **Block records** (`MV.ticket.block-record-validation`) — discovers every `planning/blocks/<BlockID>.json` file per registered repo (D65's per-block work definition) and runs seven warning-only `W_BLOCK_*` checks over each: missing/empty `why`, `description`, `out_of_scope`; a `spec_dir` that doesn't match `planning/<id>/`; a filename that doesn't match the record's own `id`; an `id` absent from that repo's `state.json` `tracks[]`; and an operator `depends_on[]` edge missing `exit` or `start`. A repo with no `planning/blocks/` directory is silent — no diagnostics, not an error. See [Architecture § Block records](../architecture.md#block-records-srcbrainblockrs--mvticketblock-record-validation) for the module and public functions.

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
| `W_STATE_SDLC_WORKFLOW_MISSING` | Warning | A block has no `sdlc_workflow` field at all — deliberately a warning, not an error, so it reports without blocking a push or a gate: 307 of 1031 blocks tripped it fleet-wide when it was introduced, and erroring would have red-gated the whole fleet at once. Paired asymmetrically with `E_STATE_SDLC_WORKFLOW_ENUM` above: a *wrong* value is an error, an *absent* one is a warning. Fix: add `sdlc_workflow` to the block record, set to one of `{none, patch, task, run, flow}` |
| `E_STATE_MODEL_ENUM` | Error | A `model` value is not in `{sonnet, gemini-pro, gemini-flash, either}` |
| `E_STATE_DATE_FORMAT` | Error | A carryover/backlog `created` / `reviewed` / `snoozed_until` value is not a valid `YYYY-MM-DD` (or RFC3339) date |
| `W_STATE_FOCUS_DRIFT` | Warning | Stored `focus` disagrees with the derivation from `tracks[]`; exit code is unchanged |
| `W_STATE_CARRYOVER_STALE` | Warning | A `carryover[]` entry has aged past its per-`kind` `[attention]` threshold and is not snoozed; exit code is unchanged |
| `W_STATE_BACKLOG_STALE` | Warning | An HQ `backlog[]` `idea`/`ready` node has aged past the `[attention]` backlog threshold and is not snoozed; exit code is unchanged |
| `W_DISTILL_STALE` | Warning | A D35-distilled `knowledge.md`/`memory.md` entry has aged past its `[attention]` `knowledge_days`/`memory_days` threshold (`check_distill_staleness`); exit code is unchanged |
| `E_STATE_OPERATOR_MISSING_EXIT` | Error | A `depends_on[]` `Operator` entry has an empty `exit` field |
| `E_STATE_APPROVAL_DIGEST_SHAPE` | Error | A `depends_on[]` `Approval` entry's `digest` is missing or not shaped `<algorithm>:<hex>` |
| `W_STATE_OPERATOR_STALE` | Warning | An `Operator` `depends_on` edge's owning `state.json` has an `updated` date older than the `[attention]` `operator_days` threshold (default 7); exit code is unchanged |
| `W_BLOCK_MISSING_WHY` | Warning | A `planning/blocks/<BlockID>.json` record's `why` is absent or whitespace-only; exit code is unchanged |
| `W_BLOCK_MISSING_DESCRIPTION` | Warning | A block record's `description` is absent or whitespace-only; exit code is unchanged |
| `W_BLOCK_MISSING_OUT_OF_SCOPE` | Warning | A block record's `out_of_scope` is absent or an empty list; exit code is unchanged |
| `W_BLOCK_SPEC_DIR_MISMATCH` | Warning | A block record's `spec_dir` is not `planning/<id>/`; exit code is unchanged |
| `W_BLOCK_FILENAME_ID_MISMATCH` | Warning | A block record's filename (minus `.json`) does not match its own `id`; exit code is unchanged |
| `W_BLOCK_UNKNOWN_ID` | Warning | A block record's `id` has no matching block in that repo's `state.json` `tracks[]`; exit code is unchanged |
| `W_BLOCK_OPERATOR_EDGE_INCOMPLETE` | Warning | A block record's `depends_on[]` entry with `type:"operator"` is missing `exit` or `start`; exit code is unchanged |

#### `--structure` — structural `index.md` coverage check

When `--structure` is passed, `mev` runs the full OKF schema pass first, then appends the structural coverage pass (D17 / CLAUDE.md Standing Rule 7):

1. Crawls the corpus (same registry-driven walk as the OKF pass).
2. Locates every directory's `index.md` corpus member and its direct-child corpus entries (siblings of that `index.md`; subdirectories are excluded — they are covered by their own `index.md`).
3. Extracts every markdown `path` link and `file://` URI from each `index.md` and resolves it against that `index.md`'s directory.
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

1. **Extract** — parses every corpus file for markdown `path` inline links, `file://` URIs, and `[[wikilink]]` references. External links (`http://`, `https://`, `mailto:`, `tel:`, protocol-relative `//`) and pure in-page anchors (`#section`) are unconditionally skipped.
2. **Resolve** — checks each local reference on disk:
   - Relative markdown links are resolved against the referring file's directory.
   - `file://` URIs are resolved to absolute paths.
   - `[[wikilinks]]` are matched against the set of authored `doc_id`s in the corpus.
3. **Moved-reference re-check** — reads `.brain-moves-pending` from the brain root (optional/ephemeral; if missing, no diagnostics are added). Each line is `<ISO-date> <path...>`; the pass flags any corpus reference that still targets a moved or deleted path.

The pass is **read-only** — it never mutates the corpus (D25).

| Locator | Severity | Condition |
|---|---|---|
| `E_LINK_DEAD_MARKDOWN` | Error | A markdown `path` link's resolved path does not exist on disk |
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
| `contract-freshness` | Each `[[contracts]]` entry's **canonical** doc version, extracted from `brain.toml`'s registry (`src/brain/conformance/contracts.rs`) | Each registered **consumer's** pinned copy — a markdown version line, or a named Dart constant such as `kServeApiPin` | A consumer whose pin does not **exactly equal** its canonical. Equality only, never version ordering: synapse and engine-rs are forked lineages that collide on the number 1.8.0 with different content, so "one minor ahead" would state something false. An unreadable file, an unparseable version line, or an unknown `format` reports `not-evaluable` for that edge — never `pass` |
| `surface-leak` | The files **git actually tracks** in each `[[repos]]` entry marked `public = true` | Two rules over those files: (1) a relative markdown link whose resolved target is not tracked in the same repo, (2) a dotted-quad IPv4 or `*.ts.net` hostname, minus `brain.toml`'s `[surface_allowlist]` | A link that resolves locally but 404s on GitHub (the `planning/` vault symlink and climb-out-of-repo cases), or a private infrastructure address in a file the public can read. `public` is **fail-closed**: a repo whose entry omits the flag is treated as private and never scanned |

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

