# CLAUDE.md — mev

A Rust CLI tool (`mev`) that parses and validates Markdown/MDX against a pluggable set of content
schemas. Two consumers today behind one `ContentValidator` core: **learn-ai content** (frontmatter +
JSON struct validation, link checking, code-block linting, watch-mode hot-reload for
learn-agentic-ai.com) and **Bastion Brain OKF** (OKF YAML frontmatter across the company-brain repo,
gating docs before the RAG index). See `planning/master-plan.md` — Phase 2 (the Brain OKF validator)
is the current priority.

## Before you start

- **Strategic context:** `planning/context.md` (read first) → `planning/status.md` (current state)
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


<!-- BEGIN BEADS INTEGRATION v:1 profile:minimal hash:7510c1e2 -->
## Beads Issue Tracker

This project uses **bd (beads)** for issue tracking. Run `bd prime` to see full workflow context and commands.

### Quick Reference

```bash
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --claim  # Claim work
bd close <id>         # Complete work
```

### Rules

- Use `bd` for ALL task tracking — do NOT use TodoWrite, TaskCreate, or markdown TODO lists
- Run `bd prime` for detailed command reference and session close protocol
- Use `bd remember` for persistent knowledge — do NOT use MEMORY.md files

**Architecture in one line:** issues live in a local Dolt DB; sync uses `refs/dolt/data` on your git remote; `.beads/issues.jsonl` is a passive export. See https://github.com/gastownhall/beads/blob/main/docs/SYNC_CONCEPTS.md for details and anti-patterns.

## Session Completion

**When ending a work session**, you MUST complete ALL steps below. Work is NOT complete until `git push` succeeds.

**MANDATORY WORKFLOW:**

1. **File issues for remaining work** - Create issues for anything that needs follow-up
2. **Run quality gates** (if code changed) - Tests, linters, builds
3. **Update issue status** - Close finished work, update in-progress items
4. **PUSH TO REMOTE** - This is MANDATORY:
   ```bash
   git pull --rebase
   git push
   git status  # MUST show "up to date with origin"
   ```
5. **Clean up** - Clear stashes, prune remote branches
6. **Verify** - All changes committed AND pushed
7. **Hand off** - Provide context for next session

**CRITICAL RULES:**
- Work is NOT complete until `git push` succeeds
- NEVER stop before pushing - that leaves work stranded locally
- NEVER say "ready to push when you are" - YOU must push
- If push fails, resolve and retry until it succeeds
<!-- END BEADS INTEGRATION -->
