# GEMINI.md — mev

A Rust CLI tool (`mev`) that parses and validates Markdown/MDX against a pluggable set of content
schemas. Two consumers today behind one `ContentValidator` core: **learn-ai content** (frontmatter +
JSON struct validation, link checking, code-block linting, watch-mode hot-reload for
learn-agentic-ai.com) and **Bastion Brain OKF** (OKF YAML frontmatter across the company-brain repo,
gating docs before the RAG index). See `planning/master-plan.md` — Phase 2 (the Brain OKF validator)
is the current priority.

## Before you start

- **Strategic context:** `planning/context.md` (read first) → `planning/status.md` (current state)
- **Symlink warning:** the `planning/` directory is actually a local symlink pointing to the company brain repo's `_planning/` vault (e.g. `core/_planning/mev/`). The brain repo is responsible for tracking all planning files under Git. Do not track `planning/` in this project's public Git repository (it is gitignored).
- **Plan:** `planning/master-plan.md` — the phase/block sequence
- **Pipeline config:** `planning/harness.json` — the validation skills + UI-test config the
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
6. <!-- Add project-specific standing rules here (prompt handling, registries, deployment
   boundaries, code style, etc.). -->

## Known bugs

None known at initialization.

## Build / test / run

```bash
# install — toolchain only (Rust 1.95+); deps fetch on first build
rustup show

# build  — release binary at target/release/mev
cargo build --release

# test   — unit + integration (tests/); AUTHORITATIVE for the review verdict
cargo test

# lint/format gates (must pass before review)
cargo fmt --check
cargo clippy -- -D warnings

# run    — validate the learn-ai content tree (defaults to ../learn-ai/content/learn)
cargo run -- validate ../learn-ai/content/learn
```

> The SDLC pipeline reads its validation suite from `planning/harness.json` (not from this
> block). Keep the `<test>`/`<build>` skills here in sync with that file's
> `validation.checks[]` so humans and the pipeline run the same thing.

## Directory map

```
mev/
├── .claude/        ← Gemini skills + SDLC workflow engines
├── planning/       ← context, status, master-plan, harness.json, decisions/, <concept>/
└── <source dirs>   ← add as the project grows
```

## What NOT to touch

<!-- Reference-only code, generated files, migration history, etc. List them as they appear. -->

---

## SDLC pipeline

This project carries the curated SDLC harness. Run `/prime` to orient, then drive structured
work through `/generate-tasks → /implement → /test → /review-task → /document → /log-work`.
See `.agents/skills/README.md` for the full pipeline reference.

> **Stack note:** the SDLC engines carry no stack defaults. Point them at this project's stack
> by filling `planning/harness.json` (validation skills + optional UI-test config). Copy a
> ready-made profile from `planning/harness.examples.md` (Rust / Python / Next.js). Do **not**
> edit the `workflows/*.js` engines for stack reasons — that's what `harness.json` is for.

<!-- BEGIN:response-style -->
## Response Style

Optimize every reply for an operator scanning several concurrent agent sessions. Default to the
shortest response that fully answers. Long prose is the failure mode, not thoroughness.

**Shape**

1. **First line = the outcome.** What happened, and did it work. No preamble, no restating the ask.
2. **Then the specifics, if any** — bullets, one line each, max ~6. Facts, not narration.
3. **Last line = the ask, if any** — one question the user can answer in a word.

Ceiling for a normal turn: **~150 words / ~15 lines**. Only depth the user explicitly asked for
(a review, a design rationale, a plan document) may exceed it.

**Cut**

- Reasoning narration — how you got there, what you considered, what you almost did. Report
  conclusions; the transcript already holds the steps.
- Justifying decisions that worked out. Explain only what was non-obvious or that the user may
  want to reverse.
- Unasked-for "what's next", roadmaps, option menus, and status recaps.
- Tables or headings for fewer than ~4 rows/sections — a sentence or bullets is faster to read.
- Self-assessment and stage direction: "the finding that reframes everything", "worth your
  attention", "one thing I want to flag", praise, hedging, apology.
- Re-explaining anything already in a file you just wrote. Link the path instead.

**Keep — these earn their space**

- Failures, blocks, and anything not matching what was asked: say it first, plainly, with the
  real error text.
- Assumptions the user might reject, and decisions that need their call.
- Security, data-loss, or money implications.
- Exact identifiers where they *are* the content: `src/serve/handlers/attention.rs:101`, a
  version, an error code. Never a paragraph describing what a one-line reference would say.

**Register**

Plain English for status, decisions, and trade-offs. Technical depth only where it changes what
the user does next. One idea per sentence; no stacked em-dash asides.
<!-- END:response-style -->
