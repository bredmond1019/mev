---
type: Plan
title: markdown-engine-validator Master Plan
description: Strategic roadmap and phase specifications for markdown-engine-validator — a general Markdown validator with two consumers (learn-ai content and Bastion Brain OKF frontmatter).
---

# markdown-engine-validator — Master Plan

*Living document. Created 2026-06-18. Reframed 2026-06-26 from a learn-ai-only validator to a
general Markdown validator with two consumers (see "The Reframe" below).*

## The Goal, Stated Plainly

A Rust CLI tool (`mev`) that parses and validates Markdown/MDX against a pluggable set of content
schemas. It has **two consumers today**, behind one shared validation core:

1. **learn-ai content** — frontmatter validation, JSON struct validation, link checking, and (later)
   code-block linting and watch-mode hot-reload for learn-agentic-ai.com lessons.
2. **Bastion Brain OKF** — validate the OKF YAML frontmatter on every `.md` file across the company-brain
   repo before it lands in the RAG indexer.

"Ready" for each consumer is its own checkpoint: for learn-ai, a pass that is a *superset of*
`learn-ai/scripts/validate-content.ts` plus the cross-file integrity checks the TS script can't do; for
the Brain, a `mev validate-brain` that runs green as a pre-flight gate before a `--rebuild` of the Brain
RAG index.

## The Reframe (2026-06-26)

`mev` was originally scoped to validate learn-ai content only. The Bastion Brain now requires that every
`.md` file carry correct **OKF frontmatter** (governed by brain decision D27; schema in the brain's
`docs/okf-frontmatter.md`) before it is indexed by the Python orchestrator's `index_brain.py`. Docs with
missing or malformed frontmatter index poorly or not at all, so we want a pre-index gate.

The decision (after reviewing the codebase) was **extend, not rebuild**: the repo and binary are already
generically named, the generic core (`Diagnostic`/`Report`/`Severity`, `extract_frontmatter`,
`is_kebab_case`, the `clap` shell, the temp-dir fixture harness) is cleanly isolated, and the learn-ai
coupling is quarantined to `crawl.rs::classify()` and the `ModuleMeta`/`PathMeta`/`MdxFrontmatter`
validators. A rebuild would re-derive tested infrastructure to escape coupling that is already isolated.
So the Brain validator slots in as a **parallel crawl + validator behind a new `ContentValidator` trait**
— not a retrofit of the learn-ai classifier. The concrete trigger is **Block H of the brain's
`brain-rag-improvements` plan** — the first live RAG `--rebuild` against Mac Mini Postgres — which wants
`mev validate-brain ~/Dev/agentic-portfolio` runnable as its pre-flight check.

## The Destination

`mev` as the single validation entry point for the Bastion practice's Markdown corpus, and a portfolio
artifact demonstrating idiomatic Rust CLI design. Long-term it can promote to a subcommand of the personal
Rust ops CLI (`bastion validate ...`). The load-bearing idea: **one `Diagnostic` currency, many content
schemas** — adding a third consumer (blog, another repo) is a new `ContentValidator` impl, not a fork.

## Architecture / Design Overview

```
                         ┌─────────────────────────────┐
   mev validate  ───────▶│ LearnAiValidator            │──┐
                         │  crawl: paths/<id>/modules/ │  │
                         │  validate: Module/Path/Mdx  │  │
                         └─────────────────────────────┘  │   ┌──────────────┐
                                                          ├──▶│  Report      │
                         ┌─────────────────────────────┐  │   │  Vec<Diag>   │──▶ human / --json
   mev validate-brain ──▶│ BrainValidator              │──┘   └──────────────┘
                         │  crawl: all *.md (skip git) │
                         │  validate: OKF frontmatter  │
                         └─────────────────────────────┘
        shared:  Severity · Diagnostic · Report · extract_frontmatter · is_kebab_case · non_empty
```

`ContentValidator` is an associated-type trait (`type Item; crawl(); validate_item(); run()` driver) —
`main.rs` selects the concrete validator per subcommand, so static dispatch suffices. The generic core
stays free of any consumer's domain types; each consumer owns its own crawl, item type, and validators.

---

## Phase 0 — Foundation

### Block A — Foundation setup — **Done**
- **What:** Configure the environment, scaffold the project skeleton, and verify the toolchain.
- **Why:** Establish a clean, reproducible starting point before any feature work.
- **Status:** `mev` binary scaffolded; `clap` CLI + `Diagnostic`/`Report` lib; smoke tests; all harness
  gates green.

---

## Phase 1 — Core: learn-module validation

First shippable feature set. Every block ships with tests against real fixtures (good modules + deliberately
broken ones). The bar is a *superset of* `learn-ai/scripts/validate-content.ts` (see D2). Universal currency
is the `Diagnostic` (`error` → exit 1, `warning` → exit 0); only the reporter prints.

### Block B — Crawl & classify — **Done**
- **What:** `walkdir` the content root; classify each file as `learn-module-json`, `path-metadata-json`,
  or `module-mdx`. Build a `Corpus` grouped by path-id / module-id. Filename-convention checks (no spaces,
  lowercase, modules match `^\d{2}-[a-z0-9-]+\.(json|mdx)$`).
- **Status:** `walkdir` + `Corpus`; filename conventions; tests green.

### Block C — Frontmatter & JSON struct validation — **Done**
- **What:** deserialize module `.json` into a strict `ModuleMeta`; validate enums (`difficulty`, section
  `type`, `level`), `duration` format, kebab-case `id`. Path `metadata.json` → `PathMeta`. MDX frontmatter
  parsed as real YAML (`title, description, duration, difficulty, lastUpdated`).
- **Status:** all tasks (1–7) complete; serde deserialization, required-field/enum/format checks,
  fixture-driven good+broken tests; all gates green.

### Block D — Cross-file integrity (learn-ai differentiator) — **Not started**
- **What:** pair existence (`.json` ↔ `.mdx`); the **anchor-slice contract** (each
  `content.source = "<file>.mdx#<anchor>"` resolves to a file containing `## …{#<anchor>}`); ID coherence;
  callout types ∈ `info|warning|success|error`.
- **Acceptance:** a renamed anchor in a fixture is flagged here while the TS script stays silent.
- **Priority note:** deeply learn-ai-specific. **Deprioritized** below Phase 2 now that the Brain OKF use
  case is the immediate driver.

### Block E — pt-BR parity & reporter polish — **Not started**
- **What:** each EN module requires a `pt-BR/` mirror with the identical filename; flag orphans. Finalize the
  reporter: grouped-by-file ANSI human output + `--json` for CI; correct exit codes.
- **Acceptance:** `mev validate ../learn-ai/content/learn` is green on the current corpus and reproduces every
  TS-script error plus anchor/pair/parity findings.
- **Note:** the `--json` reporter is pulled forward into **Block I** (the Brain RAG indexer needs it); Block E
  then only adds the ANSI grouping + pt-BR parity.

---

## Phase 2 — Generalize: `ContentValidator` trait + Brain OKF validation *(the current priority)*

Introduce the shared abstraction and the second consumer. This is where `mev` stops being "the learn-ai
validator" and becomes a general Markdown validator. Each block ends with `cargo fmt --check`,
`cargo clippy -- -D warnings`, and `cargo test` green, and **must keep the existing learn-ai tests passing**.

### Block F — `ContentValidator` trait + shared core
- **What:** Add the associated-type `ContentValidator` trait (`crawl` + `validate_item` + default `run`
  driver). Extract `extract_frontmatter`, `is_kebab_case`, `non_empty` (with their unit tests) into a
  `shared` module. Relocate the learn-ai code behind a `LearnAiValidator` (move `crawl.rs`/`meta.rs` into a
  `learn_ai/` module verbatim) and rewrite `validate()` as a thin wrapper. Public API (`ContentFile` fields,
  `validate`, `crawl`, `validate_file`) preserved via `pub use` so all existing tests pass unchanged.
- **Acceptance:** module layout refactored; the full existing test suite is green with no signature changes.

### Block G — Brain crawl
- **What:** A parallel crawl entry point: `MdFile { path, rel, stem }` + `crawl_brain(root)` that walks all
  `.md` under a root with a two-layer skip-list — a name blocklist (`target/`, `node_modules/`, `.git/`,
  ignored non-git dirs) and a **nested-git rule** that prunes any non-root directory containing its own
  `.git` (generically excludes every sub-project). The `depth() > 0` guard exempts the brain root, which is
  itself a git repo.
- **Acceptance:** unit tests prove nested-git dirs and `target/` are pruned, root-level `.md` is still found,
  and non-`.md` files are skipped.

### Block H — Brain OKF frontmatter validator
- **What:** `OkfFrontmatter` serde struct (all fields `Option`, extras tolerated). Validate: required-field
  presence (`type`, `title`, `description` — each absence its own diagnostic); controlled-vocab membership for
  `layer` (list), `project`, `status`; kebab-case `doc_id` if present; `keywords` count 3–7 (warning); missing
  frontmatter entirely → single error. `type` is presence-only (open vocab). Mirror the schema in the brain's
  `docs/okf-frontmatter.md`. Settle the `layer` scalar-vs-list question empirically against the live corpus.
- **Acceptance:** fixtures for good doc, each missing required field, bad vocab value, non-kebab `doc_id`,
  keywords out of range, and missing frontmatter emit the expected diagnostics.

### Block I — `validate-brain` subcommand + JSON reporter
- **What:** `mev validate-brain <brain-root>` (default `..`) wired to `BrainValidator`. A global `--json` flag
  emitting a machine-readable envelope (`{ validator, root, errors, warnings, diagnostics[] }`) the RAG indexer
  can consume; requires `Serialize` on `Diagnostic`/`Severity`. Update the CLI `about` text.
- **Acceptance:** `mev validate-brain ~/Dev/agentic-portfolio` reports real OKF violations, skips all
  nested-git sub-projects and `target/`; `--json` emits valid JSON usable as a pre-`--rebuild` gate.

---

## Phase 3 — Brain integrity: graph + sync *(serves the Brain; invoked by `bastion`)*

The deterministic checks that keep the Brain corpus *correct as a whole*, beyond per-file schema. Governed
by **brain decision D29** (mev is the single validation engine; `bastion validate` is a front door over it,
not a reimplementation). These are the three outer rings of validation after the schema ring (Phase 2):
graph + link integrity, structural coverage, and cross-repo sync. Each is read-only diagnostics (never
mutates the corpus — upholds D25) and `--json`-able so an agent can act on the findings. This is the mev
implementation of the brain-program's integrity/freshness work (the `hooks/README.md` `bastion validate
--integrity`, "Block K"), surfaced through `bastion`.

### Block J — Graph integrity (`related:` edges)
- **What:** Build a corpus-wide `doc_id` index (every `.md`'s `doc_id`, defaulting to filename stem). Flag
  every `related:` entry that points at a `doc_id` no document defines (a dangling edge), and flag duplicate
  `doc_id`s. This is the generalization of the learn-ai **anchor-slice contract** (Block D): "a reference
  must resolve to a real target."
- **Acceptance:** a `related:` ref to a renamed/deleted doc is flagged; duplicate `doc_id`s are flagged;
  clean corpus passes.

### Block K — Link integrity (markdown / `file://` / `[[wikilink]]`)
- **What:** Check that markdown `[text](path)` and `file:///…` links resolve to files that exist on disk,
  and that `[[wikilinks]]` (where used — e.g. memory docs) resolve to a known `doc_id`. Consume
  `.brain-moves-pending` (the post-commit delete/rename log) so a rename surfaces every now-broken reference
  to the old path.
- **Acceptance:** a link to a moved/deleted file is flagged; a `.brain-moves-pending` entry drives a
  targeted re-check of references to that path.

### Block L — Structural coverage (`index.md` ↔ directory, D17)
- **What:** Enforce CLAUDE.md Standing Rule 7 / D17: every file in a directory appears in that directory's
  `index.md` (orphan detection), and every `index.md` row points at a file that exists (dangling-row
  detection). Bidirectional.
- **Acceptance:** a new file not listed in its `index.md` is flagged; an `index.md` row for a deleted file is
  flagged.

### Block M — Sync integrity (brain cache ↔ sub-repo canonical)
- **What:** The `synced_from` watermark check (D29). For each project in a small **projects manifest**
  (`{project, path, status_file}` — the machine form of the CLAUDE.md sub-projects table), read the
  sub-repo's *live* `planning/status.md` "Last updated" and compare against the `synced_from` frontmatter on
  the brain cache doc `docs/projects/<project>.md`. `live > synced_from` ⇒ a `Sync` error ("brain cache
  stale — run `/log-work` / `/sync-status`"). Program-tracker wave tables are **excluded** (they lag by
  design). Requires a **cross-repo read mode** distinct from the corpus walk, since Block G's nested-git
  pruning makes sub-repos invisible to the normal walk.
- **Acceptance:** a sub-repo advanced past its brain cache's `synced_from` is flagged; an in-sync project
  passes; snapshot trackers are never flagged.
- **v2 hardening (additive):** content-hash watermark (catches edits that don't bump the date); structured
  `current_block` comparison (catches an exact in-place mismatch).

---

## Phase 4 — Depth / Hardening: blog + linting

`BlogValidator` as a fourth `ContentValidator` impl (additive, no rewrite): blog frontmatter
(`title, date, excerpt`), pt-BR filename parity, code-block language-tag linting, and local link/asset
existence — applied across content types.

---

## Phase 5+ — Differentiating Build

- `mev watch` — hot-reload via `notify`, re-validate changed files in milliseconds.
- `mev compile` — emit `manifest.json` (path → module → section index) the site *could* adopt to replace
  runtime file walking.

---

## Quick Reference Sequence Table

| Phase | Block | What | Why | Role in destination |
|---|---|---|---|---|
| 0 | A | Foundation setup (Rust scaffold, gates green) | Clean starting point | Enables everything downstream |
| 1 | B | Crawl & classify content tree | Know every file and its kind | Input to all learn-ai checks |
| 1 | C | Frontmatter & JSON struct validation | Catch missing/malformed fields | Superset of TS validator |
| 1 | D | Cross-file integrity (anchor-slice, pairs, ids) | Catch silent runtime failures | learn-ai differentiator *(deprioritized)* |
| 1 | E | pt-BR parity & ANSI reporter polish | Locale parity + human output | Phase 1 shippable |
| 2 | F | `ContentValidator` trait + shared core | One core, many schemas | Makes `mev` general |
| 2 | G | Brain crawl (all `.md`, skip nested git) | Enumerate the brain corpus | Input to OKF checks |
| 2 | H | Brain OKF frontmatter validator | Gate docs before RAG index | The brain use case |
| 2 | I | `validate-brain` subcommand + `--json` | Runnable pre-`--rebuild` gate | Brain shippable |
| 3 | J | Graph integrity (`related:` edges resolve) | Catch dangling/duplicate doc_ids | Brain correctness (D29) |
| 3 | K | Link integrity (markdown/`file://`/`[[wiki]]`) | Catch dead links + moved files | Brain correctness (D29) |
| 3 | L | Structural coverage (`index.md` ↔ dir, D17) | Catch orphan files + dangling rows | Brain correctness (D29) |
| 3 | M | Sync integrity (`synced_from` watermark) | Catch brain↔sub-repo status drift | Brain correctness (D29) |
| 4 | — | Blog validation + code-block/link linting | Cover a fourth content type | Whole-tree coverage |
| 5+ | — | `watch` (hot-reload) + `compile` (manifest.json) | Speed + precompiled index | Differentiating build |

---

*Sequenced by dependency and competence, not calendar. When life gets in the way, pick up where you left
off. Phase 2 is the current priority (Brain OKF gate); Phase 3 (Brain integrity, D29) follows; Phase 1
Blocks D–E resume after.*
