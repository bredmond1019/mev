# CLAUDE.md — mev

A Rust CLI tool (`mev`) that parses and validates Markdown/MDX against a pluggable set of content
schemas. Two consumers today behind one `ContentValidator` core: **learn-ai content** (frontmatter +
JSON struct validation, link checking, code-block linting, watch-mode hot-reload for
learn-agentic-ai.com) and **Bastion Brain OKF** (OKF YAML frontmatter across the company-brain repo,
gating docs before the RAG index). See `planning/master-plan.md` — Phase 2 (the Brain OKF validator)
is the current priority.

## Before you start

- **Strategic context:** `planning/context.md` (read first) → `planning/status.md` (current state)
- **Symlink warning:** the `planning/` directory is actually a local symlink pointing to the company brain repo's `_planning/` vault (e.g. `core/_planning/mev/`). The brain repo is responsible for tracking all planning files under Git. Do not track `planning/` in this project's public Git repository (it is gitignored).
- **Symlink traps:** `rg`/`grep`/`find` are symlink-blind by default — a search that must include `planning/` content needs `-L`/`--follow`. `git mv` fails through the symlink face ("source directory is empty") — move planning files via the real vault path (`.../_planning/<slug>/...`), never via `planning/...`. Planning changes are committed in the brain repo (`agentic-portfolio`) with an explicit pathspec, never in this repo.
- **Plan:** `planning/master-plan.md` — the phase/block sequence
- **Pipeline config:** `planning/harness.json` — the validation commands + UI-test config the
  SDLC engines run (see `planning/harness.examples.md` for ready-made stack profiles)
- **Decisions log:** `planning/decisions/` (start at `planning/decisions/index.md`) — check
  before relitigating any settled choice

## Standing rules

1. **Every new function, module, or behaviour change ships with tests.** No exceptions — this applies to ad-hoc fixes and one-off changes just as much as formal blocks/tasks. If you add or change code, add or update the tests that cover it.
2. **OKF frontmatter is required on every new `.md` file** under `docs/` and `planning/`.
   Every new file must open with a YAML frontmatter block containing:
   - **Required:** `type` (e.g. Decision, Index, Plan, Reference, Log, ProjectStatus, LocalContext),
     `title` (human-readable), `description` (one-line summary written for a searcher).
   - **Optional but strongly encouraged:** `doc_id` (kebab-case stable id; defaults to filename stem),
     `layer` (closed list — one or more of: `brain` · `engine` · `factory` · `console` · `surface` · `infra` · `business` · `content` · `meta`),
     `project` (closed slug — use `mev` for this repo; omit for genuinely cross-cutting docs),
     `status` (one of: `active` · `draft` · `deprecated` · `superseded` · `archived`),
     `keywords` (3–7 free-form topic terms),
     `related` (list of `doc_id`s of other docs this file depends on or cross-references).
   - Canonical schema and controlled vocabularies: company-brain `docs/okf-frontmatter.md`; governing decision: D27.
   - **Adding a file to a directory also requires updating that directory's `index.md`** — add a row
     for the new file. If the update changes the scope of a parent directory's `index.md`, update that
     too (propagate up the chain as needed).
3. **Sequence, not calendar** — work the order in `master-plan.md`; pick up where you left off.
4. **Decisions are append-only** — never edit a settled decision; supersede it with a new
   atomic file in `planning/decisions/` and link back.
5. **Verified identity / handles:** bredmond1019 (GitHub), learn-agentic-ai.com — treat these as the only authoritative
   identities/URLs; flag any other handle or profile link as unverified before publishing it.
6. **Use `cargo nextest run`, never plain `cargo test`, for any test run you invoke yourself
   during a task** (scoped: `cargo nextest run <module::path>`; full fast pass: `cargo nextest run
   --lib`). 25 integration-test binaries make plain `cargo test` slow. The one exception is the
   task explicitly designated to own full-suite validation for a spec — that task runs the real
   `cargo test` / `cargo build --release` gates, per `planning/harness.json`'s `command` (not
   `fastCommand`). See "Build / test / run" below for the full rationale.
7. <!-- Add project-specific standing rules here (prompt handling, registries, deployment
   boundaries, code style, etc.). -->

## Known bugs

None known at initialization.

## Build / test / run

```bash
# install — toolchain only (Rust 1.95+); deps fetch on first build
rustup show

# build  — release binary at target/release/mev
cargo build --release

# test   — fast, use this over plain `cargo test` (25 integration test binaries make full
# `cargo test` slow — nextest runs each as a parallel process instead of serially)
cargo nextest run --lib --bins

# full test — unit + integration (tests/); AUTHORITATIVE for the review verdict
cargo test

# lint/format gates (must pass before review)
cargo fmt --check
cargo clippy -- -D warnings

# run    — validate the learn-ai content tree (defaults to ../learn-ai/content/learn)
cargo run -- validate ../learn-ai/content/learn
```

> **Always prefer `cargo nextest run --lib --bins` over plain `cargo test` in this repo.** This is
> wired as the `fastCommand` on the `test` check in `planning/harness.json`, which the SDLC
> engines use for per-task (`testDepth: "fast"`) runs — reach for it manually too whenever
> iterating outside the harness. Requires `cargo-nextest` on PATH (`brew install cargo-nextest`);
> `cargo test` remains the authoritative full-suite gate.
>
> **Scope even narrower while mid-task**: `cargo nextest run <module::path>` for just the touched
> module. Only the task(s) explicitly owning full-suite validation for a spec should run the
> full `cargo test` / `cargo build --release` gates.
>
> **`sccache` is wired in via `.cargo/config.toml`** (`rustc-wrapper = "sccache"`) — caches
> compiled object code across builds so repeated compiles within an SDLC spec reuse work instead
> of recompiling from scratch. Requires `sccache` on PATH (`brew install sccache`).
>
> The SDLC pipeline reads its validation suite from `planning/harness.json` (not from this
> block). Keep the `<test>`/`<build>` commands here in sync with that file's
> `validation.checks[]` so humans and the pipeline run the same thing.

## Directory map

```
mev/
├── .claude/        ← Claude Code commands + SDLC workflow engines
├── planning/       ← context, status, master-plan, harness.json, decisions/, <concept>/
└── <source dirs>   ← add as the project grows
```

## What NOT to touch

<!-- Reference-only code, generated files, migration history, etc. List them as they appear. -->

---

## SDLC pipeline

This project carries the curated SDLC harness. Run `/prime` to orient, then drive structured
work through `/generate-tasks → /implement → /test → /review-task → /document → /log-work`.
See `.claude/commands/README.md` for the full pipeline reference.

> **Stack note:** the SDLC engines carry no stack defaults. Point them at this project's stack
> by filling `planning/harness.json` (validation commands + optional UI-test config). Copy a
> ready-made profile from `planning/harness.examples.md` (Rust / Python / Next.js). Do **not**
> edit the `workflows/*.js` engines for stack reasons — that's what `harness.json` is for.

<!-- BEGIN:response-style -->
## Response Style

You are read by an operator scanning several concurrent agent sessions. Long prose is the failure
mode, not thoroughness.

1. **First line = the outcome** — what happened, and whether it needs them.
2. **Then the specifics** — bullets, one line each, max ~6. Facts, not narration.
3. **Last line = the ask**, if there is one. One question, answerable in a word.

**Ceiling: 10 lines for a normal turn, 20 for an end-of-run report.** Only depth the operator
explicitly asked for may exceed it.

Durable detail goes to disk — the commands already require that. **Link the path; do not restate
the file.** Lead with failures, blocks, and anything that did not match the ask, in plain words with
the real error text. Cut reasoning narration, unasked-for next steps, and self-assessment.

Full rationale, the complete cut-list, and worked before/after examples: the
**`report-to-the-operator`** skill.
<!-- END:response-style -->
