# AGENTS.md — mev

A Rust CLI tool (`mev`) that parses and validates Markdown/MDX against a pluggable set of content
schemas. Two consumers today behind one `ContentValidator` core: **learn-ai content** (frontmatter +
JSON struct validation, link checking, code-block linting, watch-mode hot-reload for
learn-agentic-ai.com) and **Bastion Brain OKF** (OKF YAML frontmatter across the company-brain repo,
gating docs before the RAG index). See `planning/master-plan.md` — Phase 2 (the Brain OKF validator)
is the current priority.

## Workflow engine telemetry

**After invoking `Workflow({name: 'sdlc-task'|'sdlc-flow', ...})`, load the `stamp-workflow-run-id`
skill.** The engine script can't read its own Workflow run id back — the Workflow script API has no
`runId` global and no filesystem access — so joining a run's `sdlc-task-state.json`/
`sdlc-flow-state.json` to the exact Claude Code session transcript for cost telemetry relies on the
*invoking* agent patching the id in after the call returns. Skip this and `workflow_run_id` simply
stays `null` — a normal, expected state, never a defect to chase.

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
   --lib --bins`). The suite is ~2000 tests — 1268 unit tests under `src/` plus 743 integration
   tests in **one** binary, `tests/it` (65 modules, auto-discovered from `tests/it/main.rs`).
   Scoping is the win: `--lib --bins` skips linking and running the `it` binary entirely, and
   nextest's process-per-test model gives isolation and per-test timing that `cargo test`'s
   in-process threading does not. A `PreToolUse` hook in `.claude/settings.json` denies bare
   `cargo test`; the sanctioned escape hatch is prefixing `NEXTEST_POLICY_OVERRIDE=1`. The one
   exception is the task explicitly designated to own full-suite validation for a spec — that task
   runs the real `cargo test` / `cargo build --release` gates, per `planning/harness.json`'s
   `command` (not `fastCommand`). See "Build / test / run" below for the full rationale.
   *(This rule said "25 integration-test binaries" until 2026-09-01. That number was engine-rs's,
   and mev's own ~57 test binaries were consolidated into the single `it` binary by `373e306` on
   2026-08-27. The conclusion never changed; only the reason was wrong.)*
7. **Never `git push` this repo directly from inside it.** This repo sits in the fleet's Cargo
   path-dependency graph (`mev` -> `okf-core`; and `bastion`/`engine-rs` -> `mev`), and every
   Rust repo's CI clones its sibling path-deps at their unpinned default branch — pushing out of
   order breaks a sibling's CI on code that was actually fine (the 2026-08-18 outage: `bastion`
   red with `cannot find function lanes_brain in crate mev` purely because `mev` sat 23 commits
   unpushed on this exact repo). Route every push through the company-brain's
   `agentic-portfolio/scripts/git_push.sh --all`, which pushes the whole fleet in dependency
   order and skips a repo flagged `ci-blocked` (a Cargo dependency is red on GitHub with nothing
   queued to fix it). Branching, committing, and opening/reviewing/merging PRs to `main` locally
   are all fine from inside this repo — only the final `git push` of `main` to `origin` must go
   through that script.

## Known bugs

None known at initialization.

## Build / test / run

```bash
# install — toolchain only (Rust 1.95+); deps fetch on first build
rustup show

# build  — release binary at target/release/mev
cargo build --release

# test   — fast, use this over plain `cargo test`. Skips the tests/it integration binary
# (743 tests in 65 modules) entirely and runs each test as its own process.
cargo nextest run --lib --bins

# full test — unit + integration (tests/it); AUTHORITATIVE for the review verdict.
# A PreToolUse hook denies bare `cargo test`; prefix NEXTEST_POLICY_OVERRIDE=1 only in
# the task that owns full-suite validation for a spec.
NEXTEST_POLICY_OVERRIDE=1 cargo test

# lint/format gates (must pass before review)
cargo fmt --check
cargo clippy -- -D warnings

# consumer compile gate — compiles bastion/engine-rs TEST targets against THIS working
# tree's mev (cargo run --release, never an installed binary); perTask: false, gates at
# reconcile/push, not per-task
scripts/check_consumers.sh

# fixture suite for the consumer compile gate itself — cargo/git shimmed, no real build
scripts/test_check_consumers.sh

# same question as a verb, per-consumer, with four typed outcomes (only `broken` fails)
cargo run -- check-consumers [--consumer bastion]

# run    — validate the learn-ai content tree (defaults to ../learn-ai/content/learn)
cargo run -- validate ../learn-ai/content/learn
```

**Two skills carry the traps in mev's own query surface.** Before changing a public signature here,
load **`check-blast-radius`** — `check-consumers`' four outcomes and which of them actually fail a
run, plus why `bastion code --workspace mev` cannot see a caller in another repo. Before answering
"what should I work on next" with `frontier`/`lanes`/`blocks`, load **`pick-the-next-block`** — the
three verbs report three different meanings of "ready", and `mev blocks --repo` filters on its own
while `emit-block-graph --repo` silently does not.

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
> **`sccache` is NOT used in this repo (D57).** It was measured doing nothing: `sccache
> --show-stats` reported 1 compile request, 0 executed (rejected as non-cacheable), 0 cache hits,
> 0 misses. sccache refuses to cache incremental compilations, and cargo passes
> `-C incremental=...` for the dev/test profile — so every rustc call fell through to plain rustc
> plus a wrapper hop doing nothing. Incremental compilation is what makes this repo's
> edit-recompile loop fast; the real cost centre is LINKING, not compilation. `.cargo/config.toml`
> is gitignored (not tracked) for this reason. The one permitted route back is cold CI builds
> only: set `RUSTC_WRAPPER` together with `CARGO_INCREMENTAL=0` as environment variables — never
> in a committed config file.
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

<!-- BEGIN:session-continuity -->
## Stopping, continuing, and handing off

**Run to completion. Never stop, clear, or hand off because context is getting large.** There is no
token band, no percentage, and no "the next block would be cleaner in a fresh session." A chain runs
every block it was given; a lane that stops after one block and waits to be relaunched by hand
defeats the entire point of the run and puts the operator back in the loop after every block. If
context genuinely runs out, the harness summarizes and you keep going — that is its job, not yours.

There is exactly **one** reason to end a session early, and it is about correctness, not cost:
**something the running session depends on changed underneath it** — an engine, command file,
installed binary (`mev`, `bastion`), hook or `settings.json` edited this session, or a `CLAUDE.md`
you already read. The running session is a launch-time snapshot (base-template standing rule 10), so
it keeps producing pre-change results, which read as an unreliable agent rather than a stale
snapshot. **Name the trigger, finish the unit of work in flight, and say plainly that a fresh
session is needed.** Do not present it as a context-budget decision, and do not go looking for the
trigger as an excuse to stop.

Whenever you do hand off, write the entry point first — `status.md`, `handoff.md`, a spec's
`tasks.json`, or an orchestration-run `notes.md` — so the next agent starts from an artifact instead
of from your memory.
<!-- END:session-continuity -->
