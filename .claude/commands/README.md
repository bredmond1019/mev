# Slash Commands

Custom Claude Code commands for projects scaffolded from `base-template/`. Invoke with
`/command-name` in the prompt.

These drive **structured spec work**: a spec lives at `planning/<name>/tasks.md`, and
the pipeline takes it through implement → test → review → document → wrap-up, writing
predictably-named reports alongside it.

> **Project-agnostic harness.** The command set and `workflows/*.js` engines are fully
> stack-neutral. Validation commands, ports/routes, and the UI-test stage are all driven by
> each project's `planning/harness.json` — the engines carry no stack defaults. Copy a profile
> from `planning/harness.examples.md` to configure your project's stack.
> See `planning/decisions/D5-okf-phase-2-adopted.md` for the adoption record.

---

## SDLC Pipeline

The complete development lifecycle for structured spec work. Each step runs in a fresh agent
context, starts with `/prime`, reads the prior step's output file, and writes a
predictably-named output file.

### Phase Table

| SDLC Phase | Command | Role | Output |
|---|---|---|---|
| Session Start | `/session-recap` | Briefing: recent Log entries, where you left off, next step | chat only |
| Session Start | `/status` | Check current focus and what's in progress | chat only |
| Session Start | `/process-tasks` | Check which specs are eligible to start | chat only |
| Session End | `/wrap-up [note]` | Log work + commit; clean close without a handoff file | status.md, log.md, git |
| Session End | `/handoff [note]` | Write handoff + log work + commit; hands off to a fresh session | `planning/handoff.md`, status.md, log.md, git |
| Session End | `/close-out [--skip-coverage] [note]` | Verify coverage → patch docs → hand off; the quality-close pipeline after sdlc-run/sdlc-flow | status.md, log.md, docs/, git |
| Block Setup | `/start-block [name]` | Flip a spec to `In progress` in status.md | status.md |
| **1 — Roadmap** | `/generate-master-plan [desc]` | Author the full roadmap as canonical block definitions | `planning/master-plan.md` |
| **1 — Plan** | `/generate-tasks <name>` ·  `/generate-tasks --from <path>` | Write the full task spec from a master-plan block, **or** from a standalone block file (`--from`) | `planning/<name>/tasks.md` |
| **1 — Plan (ad-hoc)** | `/chore` · `/feature` · `/plan <desc>` | Plan ad-hoc work from a free-text description (not a master-plan block) | `planning/<prefix>-<slug>/{tasks,plan}.md` |
| **1 — Plan (opt.)** | `/breakdown <spec>` | Decompose spec into atomic, agent-executable sub-steps | `planning/<name>/breakdown.md` |
| **2 — Implement** | `/implement <spec> [N]` | Execute every task (or task N) in the spec | `planning/<name>/sdlc/reports/[taskN-]implement.md` |
| **2 — Hotfix** | `/patch` | Implement → validate → commit for low-risk single-file fixes; skips test/review/document | git history |
| **2 — Fix** | `/fix <spec> [N]` | Targeted fixes for FAIL/PARTIAL verdict; reads review report; overwrites implement report | `planning/<name>/sdlc/reports/[taskN-]implement.md` |
| **2 — Track** | `/update-task [name] <step> [note]` | Mark a step done and/or append a dated note mid-implementation | spec file (in-place) |
| **2 — Commit** | `/commit [hint]` | Stage + commit with a conventional message | git history |
| **3 — Test** | `/test <spec> [N]` | Run the project's validation suite; write snapshot | `planning/<name>/sdlc/reports/[taskN-]test.md` |
| **4 — Review** | `/review-task <spec> [N]` | Verify all criteria; run fresh tests; issue verdict | `planning/<name>/sdlc/reports/[taskN-]review.md` |
| **5 — Document** | `/document <spec> [N]` | Surgically patch `docs/`; gates on PASS verdict | `planning/<name>/sdlc/reports/[taskN-]document.md` |
| **6 — Wrap-up** | `/log-work [notes]` | Update status.md + append Log entry + sync company brain | status.md, log.md, brain `docs/projects/<slug>.md`, brain `README.md` |
| **7 — Verify run** | `/review-workflow <name> [N]` | Audit pipeline execution: reports, commits, Log, STATUS | `planning/<name>/sdlc/reports/[taskN-]workflow-review.md` |

### Pipeline Flow

```
SESSION START
  /status                          → read-only: current focus and what's next
  /process-tasks                   → read-only: which specs are eligible

BLOCK SETUP
  /start-block <spec>              → status.md

PHASE 1 — PLAN
  /generate-tasks <spec>           → planning/<spec>/tasks.md
        ↓  (optional)
  /breakdown planning/<spec>/tasks.md   → planning/<spec>/breakdown.md

PHASE 2 — IMPLEMENT
  /implement planning/<spec>/tasks.md [N]
        → planning/<spec>/sdlc/reports/[taskN-]implement.md
  (/update-task and /commit can be called any number of times during this phase)

PHASE 3 — TEST
  /test planning/<spec>/tasks.md [N]
        → planning/<spec>/sdlc/reports/[taskN-]test.md

PHASE 4 — REVIEW                   ← runs fresh tests; verdict gates next step
  /review-task planning/<spec>/tasks.md [N]
        → planning/<spec>/sdlc/reports/[taskN-]review.md

        if PASS → continue to PHASE 5 — DOCUMENT
        if FAIL/PARTIAL → PHASE 2 — FIX:
  /fix planning/<spec>/tasks.md [N]
        → planning/<spec>/sdlc/reports/[taskN-]implement.md  (overwritten)
  then repeat: /test [N] → /review-task [N] until PASS

PHASE 5 — DOCUMENT                 ← gates on PASS verdict
  /document planning/<spec>/tasks.md [N]
        → planning/<spec>/sdlc/reports/[taskN-]document.md

PHASE 6 — WRAP-UP
  /log-work [notes]                → status.md, log.md

(OPTIONAL) PHASE 7 — VERIFY RUN
  /review-workflow <spec> [N]      → planning/<spec>/sdlc/reports/[taskN-]workflow-review.md
```

### Argument Convention

Every step from Phase 2 onward takes the same form: `planning/<name>/tasks.md [N]`

Split on the last space. Trailing number = task N (scope to that task only). No number = full
spec. Use the **same `N`** throughout the pipeline — it determines all report filenames at
every step.

### Directory Layout

Each spec gets its own directory under `planning/`. All reports live in a `reports/`
subdirectory alongside the spec:

```
planning/
  <spec>/
    tasks.md          ← spec (written by /generate-tasks)
    breakdown.md      ← optional (written by /breakdown)
    sdlc/
      reports/
        implement.md         ← or task3-implement.md for task-scoped
        test.md              ← or task3-test.md
        review.md            ← or task3-review.md
        document.md          ← or task3-document.md
        workflow.md          ← or task3-workflow.md (written by /sdlc-run)
        workflow-review.md   ← or task3-workflow-review.md (written by /review-workflow)
```

### Report File Naming

Pattern: `[taskN-]{step}.md` inside `planning/<name>/sdlc/reports/`

| Step | Full-spec | Task-scoped |
|---|---|---|
| implement | `implement.md` | `task3-implement.md` |
| fix | *(overwrites implement slot)* | *(overwrites implement slot)* |
| test | `test.md` | `task3-test.md` |
| review | `review.md` | `task3-review.md` |
| document | `document.md` | `task3-document.md` |
| workflow (sdlc-run) | `workflow.md` | `task3-workflow.md` |
| workflow-review | `workflow-review.md` | `task3-workflow-review.md` |

> **Note:** `/fix` writes to the same `implement.md` slot as `/implement` — it represents the
> current state of Phase 2 work. Git history preserves prior versions.

---

## Automated & Orchestrated Pipelines

The manual Phase 1 → 7 commands above can be run end-to-end by automated workflows
(`workflows/*.js`). Invoke them like slash commands. Each runs the same pipeline stages, but
unattended.

| Workflow | Scope | Isolation |
|---|---|---|
| `/sdlc-run <name> [N]` | one task or a **full spec**, sequential | none — runs on the current branch, updates STATUS/Log directly |
| `/sdlc-task <name> N` | **one** task, parallel-safe | own git worktree; defers STATUS/Log to merge time |
| `/sdlc-block <name> [range]` | a **whole spec** as dependency-ordered waves | shared integration branch; worktrees only for genuinely parallel waves; **merges for you** |

> **Full reference with mermaid diagrams, per-stage detail, and token usage:**
> [`docs/workflows/`](../../docs/workflows/index.md) — one page per engine plus the manual lifecycle.

### `/sdlc-block` — spec-level orchestration

**Drive an entire spec to completion in one invocation** — "a more powerful `/sdlc-run`". A **pre-flight**
first guarantees a clean tree with the spec committed (auto-generates a missing `tasks.md`, commits an
uncommitted one, aborts fast if any *unrelated* file is dirty). **Analyze** then loads or derives the
dependency-ordered execution plan and snapshots baselines **once**. Each wave runs a **fresh implement
agent per task**: a width-1 wave runs **in place** on the integration branch (no worktree/merge, with
`git reset --hard` rollback on failure); a width-≥2 wave isolates each task in a worktree
(`/sdlc-task --implement-only`) and **selective-union-merges** in order. Once every task has landed, **one
consolidated back-half** (`/sdlc-run --from test`) tests → reviews → fixes → documents → wraps up the
integrated tree. Adds bounded per-task **retries** with failure **triage**, subtree-scoped escalation, and
**resume** (git + a `sdlc-block-state.json` breadcrumb) without duplicating work. See
[D23](../../planning/decisions/D23-lean-block-shared-setup.md)/[D24](../../planning/decisions/D24-consolidated-back-half.md)/[D28](../../planning/decisions/D28-sdlc-block-task-state.md).

| Arg | Meaning | Default |
|---|---|---|
| `<name>` | Required — drives every `planning/<name>/…` path. | — |
| `[range]` | Optional task selection (2nd positional **or** `--tasks`): `1-7`, `1,3,5`, `1-3,7`. | all tasks |
| `--max-retries N` | Total attempts per task before escalation. | `2` |
| `--max-wave-width W` | Max tasks run concurrently per batch (worktree waves). | `3` |
| `--verify-depth <d>` | Per-task verification: `consolidated` (per-task review off) or `consolidated+review` (one non-gating localization review per task). Overrides `harness.json` `block.verify`. | `consolidated` |

---

## Session Orientation

### `/wrap-up [note]`
Clean session close without a handoff. Runs `/log-work` (syncs status.md + appends log entry)
then `/commit`. Use this when you're done with a piece of work and don't need to hand off
to a fresh agent.

### `/handoff [note]`
Session end-of-context handoff. Writes `planning/handoff.md` (what's in flight, completed,
remaining, open questions, first command for the next agent), then invokes `/log-work` and
`/commit`. `/prime` in the next session detects the handoff file and surfaces it first.
Delete `planning/handoff.md` once the new session has consumed it.

### `/close-out [--skip-coverage] [note]`
Quality-close pipeline for the end of an `sdlc-run` or `sdlc-flow` session. Runs three
steps in sequence: **(1)** the full validation suite from `planning/harness.json` — stops
immediately if any gating check fails; **(2)** coverage gap scan — reads changed source
files, classifies gaps as adequate/non-blocking/blocking, writes minimal targeted tests for
blocking gaps and re-runs the suite to confirm; **(3)** `/update-docs --patch`; **(4)**
`/handoff` with the provided note. Pass `--skip-coverage` to skip step 2 when coverage was
already verified by a prior `/review-task`. Non-blocking gaps are noted in the handoff rather
than blocking it.

### `/session-recap`
Start-of-session briefing: reads the three most recent Log entries, status.md, the current
spec's `tasks.md`, and the `reports/` directory listing; outputs a concise briefing (under 300
words) and the exact next command. Read-only.

### `/conditional_docs [task-type]`
Routes the agent to the documentation most relevant to the current task type (feature, bug/fix,
api/endpoint, test/testing, docs/documentation). Reduces CLAUDE.md overload by surfacing only
the files needed for the task at hand. Takes an optional argument; defaults to reading
`planning/context.md` + `planning/status.md` + `planning/harness.json`.

### `/prime`
Orient to this repo at session start: reads `README.md`, `CLAUDE.md`, `planning/context.md`,
`planning/status.md`; runs `git ls-files`; summarizes the codebase, layout, focus, and standing
rules. Read-only. Embedded in every pipeline command.

### `/status`
Reads only `planning/status.md` and reports the Current focus line, what's In progress, and
what's Next. Read-only.

### `/process-tasks`
Reads `status.md`, applies sequential eligibility rules (a spec is ready only if all specs above
it are `Done`), and returns a status table. Read-only.

---

## Phase 1 — Plan

### `/generate-master-plan`
Authors (or revises) `planning/master-plan.md` — the roadmap source of truth — as a sequence of
canonical **block definitions** (`## Phase N` → `### Block X`, each with What / Why / Build notes /
Acceptance criteria) whose phase/block headers `/generate-tasks` can parse directly. Turns a
free-form planning session into the structure the rest of Phase 1 expects. `/new-project` should call
this as its post-scaffold roadmap step. See `planning/decisions/D34-adhoc-planning-seam.md`.

### `/generate-tasks`
Reads the relevant section of `planning/master-plan.md`, writes a full task spec to
`planning/<name>/tasks.md`, and **commits it** (clean tree for downstream `/sdlc-block`).
Each spec carries a **Validation Commands** block and ends with a Validate task.

**`--from <path>` mode** decomposes a single **standalone block file** (e.g. a `/plan` output)
instead of a master-plan block — for ad-hoc / experimental features kept out of the roadmap. It
derives the slug from the file's parent directory and writes `tasks.md` beside the source, then runs
the identical decomposition / pipeline-recommendation / `execution-plan.json` logic. The default
master-plan slug mode is unchanged.

### `/breakdown`
Reads a task spec and the source files each step touches, then writes a granular
`breakdown.md` — every sub-step atomic (one file, one change, one command). Both `/implement`
and `/fix` auto-detect this file and use the matching `### Step N:` section as the primary
execution guide (HOW); `tasks.md` stays authoritative for scope (WHAT).

### Ad-hoc planners — `/chore`, `/feature`, `/plan`

Entry points into Phase 1 for work that **isn't** a master-plan block. Each takes a free-text
description, researches the codebase, and writes a spec into its own `planning/<dir>/`
directory carrying the same Validation Commands block. Output feeds the rest of the pipeline
unchanged.

| Command | Use for | Writes to |
|---|---|---|
| `/chore <description>` | Maintenance / housekeeping | `planning/chore-<slug>/tasks.md` |
| `/feature <description>` | A new capability — full design, user story, phased plan | `planning/feature-<slug>/tasks.md` |
| `/plan <description>` | Anything else, scaled to complexity | `planning/plan-<slug>/plan.md` |

> Downstream commands derive report paths from the spec's **parent directory**, so a `plan.md`
> spec flows through identically to a `tasks.md` one.

`/chore` and `/feature` write a runnable `tasks.md` **directly** (the fast path). `/plan` writes a
`plan.md` that doubles as a **standalone block definition**: run it directly via `/implement`, or take
the rigorous route — `/generate-tasks --from planning/plan-<slug>/plan.md` decomposes it into a
`tasks.md` (with `execution-plan.json` + pipeline recommendation) to run on a feature branch via
`/sdlc-flow`, all **without** touching `master-plan.md`. See
`planning/decisions/D34-adhoc-planning-seam.md`.

---

## Phase 2 — Implement

### `/implement`
Runs `/prime`, reads the plan file, executes every step (or task N) following CLAUDE.md
conventions, runs the relevant Validation Commands, and writes
`planning/<name>/sdlc/reports/[taskN-]implement.md`.

### `/fix`
Reads the review report to extract every failing criterion, orients via `/prime`, and applies
targeted changes addressing only the failures. Overwrites the `implement.md` slot. Hard-errors
if the review report is absent; soft-stops if the verdict is already PASS.

### `/update-task`
Optionally marks a step done (prepends ✅) and/or appends a dated note to the spec's `## Notes`
section. Auto-detects the current spec from status.md if not given. Does not touch status.md.

### `/commit`
Inspects `git status`/`git diff --stat`, chooses a commit strategy (code-only, docs-only, or
both → two commits), drafts a conventional message, and confirms before committing. Never
pushes, never `--no-verify`, never `git add -A`.

---

## Phase 3 — Test

### `/test`
Runs `/prime`, then the project's validation suite (lint, type-check, tests, build, and any
project-specific gates), returning results as a JSON array sorted failed-first. With a spec path,
also writes `planning/<name>/sdlc/reports/[taskN-]test.md`.

> **Stack note:** the test stage runs the checks defined in `planning/harness.json`
> (`validation.checks[]`). The harness ships no stack defaults — define your project's actual
> validation commands there (copy a profile from `planning/harness.examples.md`). If the config
> is absent, the stage falls back to the spec's `## Validation Commands` section.

---

## Phase 4 — Review

### `/review-task`
Runs `/prime`, reads the `implement.md`/`test.md` reports as context, then runs a **fresh test
suite** as authoritative verification. Verdict is PASS only if all criteria are MET **and** the
fresh tests pass. Writes a review report.

---

## Phase 5 — Document

### `/document`
Gates strictly on the review verdict being PASS. Reads the implement report's **Files Created
or Modified** table to scope updates, then surgically patches only affected sections of
`docs/*.md`. Flags architecture-level changes as `NEEDS_REVIEW`. Never touches `planning/`,
`log.md`, `status.md`, or `CLAUDE.md`.

---

## Phase 6 — Wrap-up

### `/log-work`
Reads `status.md`, the current spec, and `log.md`; runs `git diff --stat`. Updates
`status.md` and appends a `log.md` entry. Prompts you to add settled choices to
`planning/decisions/` — never edits decisions directly. Also syncs the company brain
(`docs/projects/<slug>.md`, `README.md`) to match the new status.

---

## Phase 7 — Verify Run (Optional)

### `/review-workflow`
Audits a completed `/sdlc-run` pipeline execution — not the implementation, but the mechanics:
report files present and well-formed, the Test stage ran the suite, commits follow conventional
format, Log/STATUS reflect the outcome. Issues PASS/PARTIAL/FAIL and writes
`workflow-review.md`. Does **not** re-run tests — use `/review-task` for that.

---

## Block Setup & Worktree Management

### `/start-block`
Finds the target spec (defaulting to the first non-done spec), checks that all preceding specs
are `Done`, then flips it to `In progress` and updates Current focus + Last updated.

### `/init-worktree` · `/clean-worktree`
Manual entry points for the isolated-worktree lifecycle that `/sdlc-task` and `/sdlc-block`
automate. `/init-worktree` derives a branch/worktree from the spec slug and creates an isolated
sparse checkout; `/clean-worktree` **merges before delete** — fast-forward-merges the branch
into `main`, applies deferred STATUS/Log updates, then removes the worktree. Do **not** run
`/clean-worktree` for `/sdlc-block` tasks — that orchestrator merges each wave for you.

### `/update-docs [--patch] [--since <ref>]`
Documentation health sweep — audits all `docs/` files and `.claude/commands/README.md` against
the current codebase (commands, engine flags, schema fields, new decisions) and recent git
history. Produces a structured gap report: **STALE** sections, **MISSING** coverage, **NO-DOC**
(intentionally undocumented), and **CURRENT** (confirmed). Add `--patch` to apply surgical
fixes for clear-cut stale sections; without it the command is read-only. The un-gated complement
to `/document` — use for periodic doc health checks outside the pipeline.

---

## Company Brain Integration

`/log-work` automatically mirrors status updates to the parent `agentic-portfolio/` company
brain (`docs/projects/<slug>.md`, `README.md`). To run brain-level commands (briefing,
sync-status, log-decision, add-project, log-correspondence), open Claude Code in the
`agentic-portfolio/` root.
