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

- **Scope resolution (multi-root corpus).** `scope_for(rel, config) -> String` = longest-prefix match of the file's path against `brain.toml` `[[repos]]` `repo_path` entries. The root unit (`repo_path = "."`, slug `brain`) is the fallback. Scope is a **registry-driven stable slug** — never inferred from path/tier position, so moving a repo between tiers does not change any node ids.
  source: planning/archive/2.J-corpus-crawl/tasks.md · date: 2026-06-28 · supersedes: — · freshness: 2026-06-28

- **Multi-root corpus crawl.** `crawl_corpus(root, config) -> (Corpus, Vec<Diagnostic>)` returns an owned, `Serialize`-able `Corpus` (each `CorpusEntry` carries `{path, rel, stem, scope: String}`), separate from diagnostics. Corpus membership rule: a `.md` file is in the corpus iff, relative to its owning unit (longest-prefix), it is under `planning/` or `docs/`, OR it is the unit's root `README.md`/`CLAUDE.md`. Ephemeral exclusions: `handoff.md` and `_`-prefixed files. Bloat dirs pruned via `skip_dirs` from `brain.toml`. This is the D4 manifest seed — the same list the embedder should consume (D4 "what's validated == what's embedded").
  source: planning/archive/2.J-corpus-crawl/tasks.md · date: 2026-06-28 · supersedes: — · freshness: 2026-06-28

- **OKF exemption for root instruction files.** `README.md`/`CLAUDE.md` without frontmatter produce **no** OKF "missing frontmatter" error — they are valid corpus leaves (per HQ CLAUDE.md Standing Rule 6). A root file that *does* carry frontmatter (and `doc_id`) is validated normally and promoted to a graph node. `handoff.md` remains ephemeral/excluded.
  source: planning/archive/2.J-corpus-crawl/tasks.md · date: 2026-06-28 · supersedes: — · freshness: 2026-06-28

- **Graph model: nodes vs leaves, `scope:doc_id`.** Canonical node id = `scope:doc_id`. A file **with** an authored `doc_id` is a **node** (globally unique; legal `related:` target). A file **without** a `doc_id` is a **leaf** (embedded for retrieval; never uniqueness-checked; flagged `W_GRAPH_LEAF_TARGET` if named as a `related:` target). See D6 for the full id scheme and corpus rules.
  source: planning/archive/2.J-graph-integrity/tasks.md · date: 2026-06-29 · supersedes: — · freshness: 2026-06-29

- **Graph artifact: build + check separation.** `build_graph(corpus, config) -> GraphArtifact` builds an owned `GraphArtifact` wrapping `Graph{nodes: Vec<Node>, edges: Vec<Edge>}` plus lookup structures (`node_map`, `leaf_keys`). `check_graph(&artifact) -> Vec<Diagnostic>` consumes the built artifact — it does **not** re-walk the corpus. All types derive `serde::Serialize` so the validated graph is the emittable graph (D4 — Phase 3B Block R loads it into Postgres). Edge model: `Edge{from: String, to_ref: String, kind: EdgeKind::Related}` designed for typed-edge extension (no reshape needed to add `supersedes`/`depends-on`/`parent`).
  source: planning/archive/2.J-graph-integrity/tasks.md + worklog.md · date: 2026-06-29 · supersedes: — · freshness: 2026-06-29

- **`read_doc_metadata` seam (D5 forward-compat). — SUPERSEDED by the D5 extract-once entry below (Block 3B.Q removed this seam).** In `src/brain/graph.rs`, a single helper `read_doc_metadata(entry) -> DocMeta{doc_id, related}` was the **only** site that read inline Markdown frontmatter (`extract_frontmatter + OkfFrontmatter`) for graph construction. All of `build_graph`/`check_graph` routed through it.
  source: planning/archive/2.J-graph-integrity/tasks.md · date: 2026-06-29 · supersedes: — · freshness: 2026-06-29

- **Frontmatter is parsed exactly once per file — D5 extract-once.** Frontmatter is read a single time, during `crawl_corpus()`, and surfaced on `CorpusEntry.metadata` (`Option<OkfFrontmatter>`); I/O or YAML errors degrade to `None`. `build_graph`, `collect_doc_ids` (links.rs), and `build_manifest` all read `doc_id`/`related`/OKF fields from `entry.metadata` — no site re-reads frontmatter. The old `read_doc_metadata()` seam and its `RawFrontmatter` helper in `graph.rs` were **removed**. `OkfFrontmatter` now derives `Clone + Serialize` (was `Deserialize`-only) — `Serialize` so the manifest can emit metadata, `Clone` because `CorpusEntry` derives `Clone`.
  source: planning/archive/3B.Q-manifest-emit/worklog.md · date: 2026-06-30 · supersedes: `read_doc_metadata` seam entry above · freshness: 2026-07-02

- **Manifest emit (`src/brain/manifest.rs`).** `build_manifest(root, corpus) -> Manifest` maps each `CorpusEntry` to a `ManifestEntry` (rel, scope, doc_id, doc_type, title, description, layer, project, status, keywords). Exposed via `mev::manifest_brain(root)` and the `mev manifest <root>` CLI (`--pretty`; compact JSON default). Output **is** JSON — no `--json` envelope. `manifest_brain()` discards crawl diagnostics (validate_brain owns diagnostics) and returns `Err` only on hard config failure.
  source: planning/archive/3B.Q-manifest-emit/worklog.md · date: 2026-06-30 · supersedes: — · freshness: 2026-07-02

- **Graph emit (`src/brain/graph_emit.rs`).** `build_graph_export(root, &GraphArtifact) -> GraphExport{version, root, nodes, edges, leaves}` reuses `Node`/`Edge` from `graph.rs` and sorts `leaves` for deterministic output; `graph_brain(root)` is the library driver (mirrors `manifest_brain`); `mev emit-graph [--pretty]` is the CLI. Distinct from the HTML-emitting `generate-graph` subcommand. All types `Serialize` so the validated graph is the emittable graph (D4 pure-compiler).
  source: planning/archive/3B.R-graph-emit/tasks.md · date: 2026-07-01 · supersedes: — · freshness: 2026-07-02

- **Link path resolution: lexical, never `canonicalize()`.** `normalize_path` resolves link/moved-reference paths by walking components lexically, so it works on paths that no longer exist on disk — the whole point of `.brain-moves-pending` is matching deleted/moved targets, which `canonicalize()` would fail on.
  source: planning/archive/3.K-link-integrity/worklog.md · date: 2026-06-30 · supersedes: — · freshness: 2026-07-02

- **`FileUri` vs `Markdown` link resolution differ.** In both `links.rs` and `structure.rs`, `file://` link targets resolve as absolute paths (strip the scheme); `Markdown` `[text](path)` links resolve relative to the containing file's directory. In `structure.rs`, `index.md` entries are identified by `entry.path.file_name() == Some("index.md")`, not by stem.
  source: planning/archive/3.L-structural-coverage/sdlc/worklog.md · date: 2026-07-02 · supersedes: — · freshness: 2026-07-02

- **State files are discovered by absolute path off the HQ root, NOT via the corpus crawl.** `planning/state.json` files live inside gitignored, nested-git sub-repos that the corpus crawl's nested-git pruning makes invisible, so discovery follows the sync pattern: HQ-root + each tier sub-brain's `planning/state.json` (brain files), plus each `brain.toml` `[[repos]]` `path`/`planning/state.json` (leaf files). Tier sub-brain state files are found by loading the HQ `state.json` and reading its `tiers[].rollup` paths — deliberately not by adding a `tiers` section to `brain.toml` (keeps tier topology in the state graph, not duplicated in config).
  source: planning/archive/3.P-state-integrity/tasks.md · date: 2026-06-29 · supersedes: — · freshness: 2026-07-02

- **v2 state schema — the authoritative work-block DAG.** The DAG lives in `tracks[].blocks[].depends_on[]` (reusing the `BlockedBy` enum: `{type:block,repo,id,what?}` | `{type:external,what}`). `type:block` entries become graph edges (`from`=owning `repo:id`, `to`=`entry.repo:entry.id`); `type:external` entries are leaves, never edges/nodes. `focus.blocked_by[]` is **no longer** an edge source in v2 — focus/rollup are derived views. Authored block `status` ∈ `{open,in_progress,closed}`; `blocked` is derived, never authored. `wave` (int) is the execution-order rank orthogonal to track grouping; `origin:{type:backlog,slug}` marks promoted blocks. HQ-only `backlog[]` nodes key on `slug`, status ∈ `{idea,ready,promoted}`, carry their own `depends_on[]` + optional `block` promotion pointer.
  source: planning/archive/3.P2-state-graph-validation/tasks.md · date: 2026-06-30 · supersedes: v1 `state.json` model (Block 3.P) · freshness: 2026-07-02

- **`ready_order` / `detect_cycles` (in `src/brain/state.rs`).** `ready_order(graph, files)` is a standalone reusable topo/readiness function — a block is *ready* iff every `type:block` dep is `closed` AND it has zero `type:external` deps; ready+`open` blocks order by `wave`, tiebreak track order then array order, with `wave=None` treated as `i64::MAX` (last). Built standalone (not buried in a check) because `emit-state` (3B.T) serializes this exact ordering. `detect_cycles` (DFS back-edge → `E_STATE_CYCLE` naming the path) walks only `BlockedBy` edges; `CrossRepo` edges are excluded from the DAG.
  source: planning/archive/3.P2-state-graph-validation/tasks.md · date: 2026-06-30 · supersedes: — · freshness: 2026-07-02

- **Derivation is single-sourced across validator and emitter.** `derive_focus` is the one derivation shared by both the validator (`check_focus_drift` delegates to it, then set-compares in place) and the emitter (`plan_state_json`), so the drift check and the emit can never disagree; same pattern for `derive_rollup`/`derive_cross_repo`. `emit-state --write` is by construction the fixed point of the drift check (run it, then `validate-brain --state` yields zero `W_STATE_FOCUS_DRIFT`/`W_STATE_ROLLUP_DRIFT`). Never reimplement derivation inline in emit.
  source: planning/archive/3B.T-state-table-rollup-emit/tasks.md · date: 2026-06-30 · supersedes: — · freshness: 2026-07-02

- **`validate_brain_sync` / `--sync` watermark check.** `validate_brain_sync(root) -> anyhow::Result<Report>` runs the normal OKF schema pass plus `check_sync(root, &config)` per `[[repos]]` entry. Source watermark = `timestamp` in `status_file`; cache watermark = `synced_from` in `cache_doc`; both paths from `brain.toml` `[[repos]]`, resolved relative to HQ root. Watermarks parsed strictly as RFC3339 (`DateTime::parse_from_rfc3339`); date-only strings (`"2026-06-27"`) are rejected as `E_SYNC_WATERMARK_MALFORMED`. CLI flag: `--sync` on `validate-brain`.
  source: planning/archive/block-n-sync-watermark/tasks.md · date: 2026-06-28 · supersedes: — · freshness: 2026-06-28

- **`--graph` and `--sync` are mutually exclusive by precedence.** When both flags are supplied, `--graph` wins (graph is a strict superset of the OKF schema pass that `--sync` also runs). No error is produced for combining them — simpler UX.
  source: planning/archive/2.J-graph-integrity/sdlc/worklog.md · date: 2026-06-29 · supersedes: — · freshness: 2026-06-29

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

- **`validate-brain` flag dispatch precedence: `--links > --structure > --state > --graph > --sync > default`.** The `ValidateBrain` subcommand runs exactly one check mode via a mutually-exclusive `else if` ladder (documented in the `--structure` flag doc comment in `src/main.rs`); flags do **not** compose. New single-check flags are prepended at the top of the ladder rather than appended.
  source: planning/archive/3.L-structural-coverage/sdlc/worklog.md · date: 2026-07-02 · supersedes: — · freshness: 2026-07-02

- **Wikilinks are scope-agnostic bare `doc_id`s — unlike `related:` edges (`scope:doc_id`).** `[[name]]` matches the bare authored `doc_id` set (matching memory-doc usage). Consequently WikiLink targets are excluded from `.brain-moves-pending` path scanning — only `Markdown` and `FileUri` links are path-resolved against moved paths.
  source: planning/archive/3.K-link-integrity/worklog.md · date: 2026-06-30 · supersedes: — · freshness: 2026-07-02

- **`ManifestEntry` field-naming + path portability.** The OKF `type` field is stored as Rust field `doc_type` with `#[serde(rename = "doc_type")]` — avoids the `type` keyword while keeping `doc_type` in JSON (no hidden rename). `rel` paths are normalized to forward slashes via `replace(MAIN_SEPARATOR, '/')` for cross-platform JSON portability.
  source: planning/archive/3B.Q-manifest-emit/worklog.md · date: 2026-06-30 · supersedes: — · freshness: 2026-07-02

- **Generated views are spliced between sentinel comments** `<!-- BEGIN generated:<marker> -->` … `<!-- END generated:<marker> -->` (e.g. marker `wave-table` in `master-plan.md`). `splice_generated` replaces only the text between them, preserves every non-sentinel line verbatim, is idempotent, and preserves the original's trailing-newline behaviour. Missing/unbalanced sentinels → `EmitError::MissingSentinel`, surfaced as a soft `W_EMIT_NO_SENTINEL` warning — never invent sentinels into arbitrary prose.
  source: planning/archive/3B.T-state-table-rollup-emit/tasks.md · date: 2026-06-30 · supersedes: — · freshness: 2026-07-02

- **`Diagnostic` has only `error`/`warning` severities — no info.** Info-flavoured emit codes (`I_EMIT_WROTE`, `W_EMIT_DRY_RUN`) are therefore Warning severity so they surface in the human + `--json` reporter without failing the exit code; only `E_EMIT_WRITE_FAILED` (real IO failure) is Error severity.
  source: planning/archive/3B.T-state-table-rollup-emit/breakdown.md · date: 2026-06-30 · supersedes: — · freshness: 2026-07-02

- **v2 state-graph diagnostic codes** (in `validate_brain_state`): `E_STATE_CYCLE` (depends_on cycle, exit 1), `E_STATE_AUTHORED_BLOCKED` (track block authored `status:"blocked"`), `E_STATE_STATUS_INCONSISTENT` (`closed` block with a non-`closed` `type:block` dep), `E_STATE_DANGLING_PROMOTION` (`promoted` backlog node whose `block` pointer resolves to nothing), `W_STATE_FOCUS_DRIFT` (stored `focus` disagrees with derivation — warning, exit 0). Backlog dangling deps reuse `E_STATE_DANGLING_BLOCKED_BY`; bad backlog status reuses `E_STATE_SCHEMA_BAD_STATUS`.
  source: planning/archive/3.P2-state-graph-validation/tasks.md · date: 2026-06-30 · supersedes: — · freshness: 2026-07-02

- **`state-schema.md` lives in the `core` repo, not the `mev` worktree.** Schema/doc edits there are committed in `core` separately — they cannot ride along in a mev branch commit. Any mev block whose spec touches `state-schema.md` produces a cross-repo commit split; account for it in review/close-out rather than treating it as missing work.
  source: planning/archive/3B.U-brain-rollup-tier-scoping/tasks.md · date: 2026-07-02 · supersedes: — · freshness: 2026-07-02

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

- **Bare `related:` references resolve within scope; qualified cross-scope.** A bare `doc_id` in a `related:` list resolves within the referrer's own scope. A bare ref that names another scope's `doc_id` is **not** resolved cross-scope — it is flagged `E_GRAPH_DANGLING_RELATED`. Only `scope:doc_id` qualified refs resolve cross-scope.
  source: planning/archive/2.J-graph-integrity/tasks.md · date: 2026-06-29 · supersedes: — · freshness: 2026-06-29

- **Graph diagnostic locators.** `E_GRAPH_DUPLICATE_DOC_ID` is the diagnostic `locator` string for duplicate node checks. For dangling-edge and leaf-target diagnostics, the locator is the string `"related"` (not the vocabulary code names `E_GRAPH_DANGLING_RELATED` / `W_GRAPH_LEAF_TARGET` — those are documentation labels).
  source: planning/archive/2.J-graph-integrity/sdlc/worklog.md · date: 2026-06-29 · supersedes: — · freshness: 2026-06-29

- **`check_sync` short-circuits per repo.** For each `[[repos]]` entry, `check_sync` emits the first applicable error and moves to the next repo — it does **not** accumulate multiple errors for the same repo. This mirrors how OKF validation short-circuits on read failure.
  source: planning/archive/block-n-sync-watermark/sdlc/worklog.md · date: 2026-06-28 · supersedes: — · freshness: 2026-06-28

---

*Durable knowledge. For episodic notes see `memory.md`; for the chronological narrative see the
root `log.md`.*
