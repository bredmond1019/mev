---
type: Reference
title: mev Knowledge
description: Distilled, durable knowledge for mev — how it works, conventions, and an architecture digest.
doc_id: knowledge
layer: [factory]
project: mev
status: active
keywords: [knowledge, conventions, architecture, semantic memory, durable]
related: [context, status, memory, planning-index]
---

# Knowledge — mev

Distilled, **durable** project knowledge: how the system works, the conventions it follows, and an
architecture digest. This is *semantic memory* at repo scope — the things a new agent should read
to understand the project, kept current as the design settles.

Seed it from `context.md`, the decision record, and what you learn while building. Keep entries
durable (how things work), not episodic (what happened) — episodic notes go in `memory.md`, settled
choices go in `decisions/`. Each entry promoted from the cold archive tier carries provenance
(D35 format: claim · source · date · supersedes · freshness).

## How it works

_Architecture digest — the main components and how they fit together._

- **Two consumers, one engine.** mev ships a `ContentValidator` trait (associated type `Item`) with two implementations: `LearnAiValidator` (learn-ai content tree) and `BrainValidator` (company-brain OKF docs). All validation logic routes through this trait; `validate()` and `validate_brain()` are thin wrappers.
  source: log.md (2026-06-26 Block 2.F entry) · date: 2026-06-26 · supersedes: — · freshness: 2026-06-27

- **`mev validate-brain <root>`** is the Brain-OKF subcommand (default root `..`). It accepts a global `--json` flag that emits a machine-readable `JsonReport` envelope. `Severity` is lowercase-serialized via `serde rename_all`.
  source: log.md (2026-06-26 Block 2.I entry) · date: 2026-06-26 · supersedes: — · freshness: 2026-06-27

- **`brain.toml` walk-up resolver.** `find_brain_config` walks up from the corpus root to find `brain.toml`. If none is found, built-in defaults apply. `load_brain_config` parses it; the result is threaded through `validate_brain` in `lib.rs`. This mirrors how `.eslintrc` / `pyproject.toml` travel with their corpus.
  source: log.md (2026-06-27 Block 2.M entry) · date: 2026-06-27 · supersedes: D3 `.mev.toml` proposal · freshness: 2026-06-27

- **Vocab validation is config-driven.** `is_valid_layer`, `is_valid_status`, `is_valid_project` no longer contain hardcoded string arrays in production source; they read from the `BrainConfig` loaded by Block 2.M. Corpus-specific controlled-vocab sets live in `brain.toml` under `[vocab]`.
  source: log.md (2026-06-27 Block 2.M Task 3) · date: 2026-06-27 · supersedes: hardcoded arrays in `src/brain/okf.rs` · freshness: 2026-06-27

- **Crawl skip-list from `brain.toml`.** `crawl_brain` reads `skip_dirs` from config; entries can be leaf names (e.g. `target`) or relative paths (e.g. `planning/archive`). The helper `is_blocklisted_name` accepts a relative-path parameter so path-style entries prune correctly.
  source: log.md (2026-06-27 Block 2.M Task 6) · date: 2026-06-27 · supersedes: hardcoded leaf-name-only blocklist · freshness: 2026-06-27

- **`is_decision_id` / `is_valid_doc_id` pattern.** `src/brain/okf.rs` defines `is_decision_id()` accepting the Brain's `D<n>-…` convention (e.g. `D3-corpus-config-system`) and `is_valid_doc_id()` delegating to it alongside standard kebab-case. These remain the pattern-matching engine even as the vocab values move to config.
  source: log.md (2026-06-26 crawl-hardening entry) · date: 2026-06-26 · supersedes: — · freshness: 2026-06-27

- **OKF frontmatter validation rules.** `OkfFrontmatter` is a serde struct (all fields `Option`; `layer` is `Option<Vec<String>>`; extra fields tolerated). Required: `type`, `title`, `description` — each missing field emits a separate `error` with a precise locator. Controlled-vocab errors fire only when the field is present. `doc_id` must be kebab-case (or a decision-id). `keywords` count outside 3–7 emits a `warning`, not an error.
  source: log.md (2026-06-26 Block 2.H entry) · date: 2026-06-26 · supersedes: — · freshness: 2026-06-27

- **Brain crawl pruning.** `crawl_brain` uses `filter_entry`-based directory pruning with two helpers: `is_blocklisted_name` (prunes dirs like `target/`, `node_modules/`, `.git/`, `.claude/`, `.repo-backups/`, `.agent/`) and `has_nested_git` (prunes nested git repos at depth > 0, preventing accidental descend into sub-project directories).
  source: log.md (2026-06-26 Block 2.G entry) · date: 2026-06-26 · supersedes: — · freshness: 2026-06-27

- **learn-ai content tree layout.** The classifier must handle: `paths/<path-id>/metadata.json` → `PathMetadataJson`; `paths/<path-id>/modules/<NN-slug>.json` → `LearnModuleJson`; `paths/<path-id>/modules/<NN-slug>.mdx` → `ModuleMdx`; pt-BR mirror nests under `paths/<path-id>/pt-BR/`. Everything outside `paths/` (schemas, shared, top-level `.md`) is skipped.
  source: planning/archive/phase1-blockB/breakdown.md · date: 2026-06-18 · supersedes: — · freshness: 2026-06-27

- **Silent failure mode mev was built to catch.** The learn-ai server (`lib/content/learning/modules.server.ts`) slices each section with the regex `(## .*\{#<anchor>\}[\s\S]*?)(?=\n## |$)`. A missing anchor silently renders "Content for section X not found" at runtime — no build error, no TS validator warning. mev's anchor-slice contract check (Phase 1, Block D) is the primary differentiator over the existing `scripts/validate-content.ts`.
  source: planning/decisions/D2-scope-and-sequence.md · date: 2026-06-18 · supersedes: — · freshness: 2026-06-27

## Conventions

_Naming, patterns, and standing choices specific to this project._

- **Scope: superset of the TS script.** mev targets a strict superset of `scripts/validate-content.ts` (learn-only, substring frontmatter checks). The TS script is the retirement target once mev is proven and wired in as a pre-build gate.
  source: planning/decisions/D2-scope-and-sequence.md · date: 2026-06-18 · supersedes: — · freshness: 2026-06-27

- **`BTreeMap` everywhere, not `HashMap`.** Corpus iteration order must be deterministic — fixture tests assert on order and CI must be reproducible. Use `BTreeMap`/sorted accessors throughout.
  source: planning/archive/phase1-blockB/breakdown.md · date: 2026-06-18 · supersedes: — · freshness: 2026-06-27

- **No `regex` crate.** Filename pattern checks (e.g. `^\d{2}-[a-z0-9-]+\.(json|mdx)$`) are implemented by hand with char-class checks. The `Cargo.toml` does not carry a `regex` dependency; adding one is out of convention.
  source: planning/archive/phase1-blockB/breakdown.md · date: 2026-06-18 · supersedes: — · freshness: 2026-06-27

- **`validate()` public contract.** The top-level `validate(root: &Path) -> anyhow::Result<Report>` signature is preserved across all blocks so `src/main.rs` stays untouched. `Corpus` is built inside `validate()` and bound (`_corpus`) ready for downstream blocks to consume.
  source: planning/archive/phase1-blockB/breakdown.md · date: 2026-06-18 · supersedes: — · freshness: 2026-06-27

- **Harness gates (four, non-negotiable).** Every block must pass: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, `cargo build --release`. No exceptions.
  source: CLAUDE.md (standing rules) · date: 2026-06-18 · supersedes: — · freshness: 2026-06-27

- **D3 `.mev.toml` superseded by `brain.toml`.** The original plan for a per-corpus `.mev.toml` (D3) is retired. The shared `brain.toml` at HQ root is the corpus config, consumed by both `mev validate-brain` and `index_brain.py`. Walk-up resolution and vocab/crawl surface are preserved from the D3 spec; only the filename and "each consumer carries its own" model changed.
  source: planning/decisions/D3-corpus-config-system.md · date: 2026-06-27 · supersedes: D3 draft · freshness: 2026-06-27

- **Phase sequence.** Phase 1 = learn-ai content validation (frontmatter, pair existence, anchor-slice). Phase 2 = Brain OKF validation (`validate-brain` + `brain.toml` config). Phase 3+ = graph/link/structural integrity checks. Compile and watch-mode deferred to Phase 3+.
  source: planning/decisions/D2-scope-and-sequence.md + log.md · date: 2026-06-18 · supersedes: — · freshness: 2026-06-27

## Gotchas

_Non-obvious constraints, sharp edges, and hard-won lessons._

- **Path-style `skip_dirs` entries were silently ignored.** Before Block 2.M Task 6, `is_blocklisted_name` only compared the leaf name of a path component. Entries like `planning/archive` in `skip_dirs` silently did nothing. The fix extended the helper to accept a relative-path parameter and check the full path suffix. Any future `skip_dirs` logic must handle both forms.
  source: log.md (2026-06-27 Block 2.M Task 6) · date: 2026-06-27 · supersedes: — · freshness: 2026-06-27

- **Filename violations do not drop files from the corpus.** A file that fails a filename check still gets pushed to `corpus.files`. Downstream blocks still see it. This matches the TS validator's `validateFileName` behavior.
  source: planning/archive/phase1-blockB/breakdown.md · date: 2026-06-18 · supersedes: — · freshness: 2026-06-27

- **`non_empty` returns the original string, not a trimmed copy.** A misleading docstring was fixed post-Block 2.F review. The function checks non-emptiness only; callers that need trimming must trim themselves.
  source: log.md (2026-06-26 Block 2.F close-out) · date: 2026-06-26 · supersedes: — · freshness: 2026-06-27

- **`out of scope` content directories.** `content/summaries/` and `content/youtube-transcripts/` are source material, not in the build pipeline, and explicitly out of scope (D2). Do not add validation rules for them.
  source: planning/decisions/D2-scope-and-sequence.md · date: 2026-06-18 · supersedes: — · freshness: 2026-06-27

---

*Durable knowledge. For episodic notes see `memory.md`; for the chronological narrative see the
root `log.md`.*
