# Generate Tasks — Generate a task spec for a specified phase and block.

## Variables

$ARGUMENTS — the spec's `planning/` directory name (its phase-dotted slug),
             e.g. `<spec-slug>` or `2.1-learn-paths-structural-fixes`.
             New master-plan specs follow the `P.N-slug` convention (see
             `planning/index.md` → *Task directory naming convention*); ad-hoc work uses
             `/chore`, `/feature`, or `/plan` instead.
             Required. If omitted, stop and say: "Usage: /generate-tasks <P.N-slug>  (e.g. <spec-slug>)"

## Instructions

1. Run `/prime` to orient to the repo (standing rules, architecture).

2. Parse `$ARGUMENTS` to extract the phase number and block/project identifier
   (e.g. `phase0-blockC` → phase 0, block C).
   - Accept any of these forms: `phase0-blockC`, `phase0blockC`, `0-C`, `Phase 0 Block C`.
   - If the argument cannot be parsed into a phase + block, stop and explain the expected format.

3. Check whether a spec already exists at `planning/phaseN-blockX/tasks.md` (using the
   normalized directory form, e.g. `planning/<spec-slug>/tasks.md`).
   - If it exists, read it and report: "Spec already exists at <path>. Overwrite? (re-run with
     `--force` appended to overwrite, or run `/breakdown <path>` to decompose it instead.)"
   - If `$ARGUMENTS` contains `--force`, proceed and overwrite.

4. Read ONLY the relevant section for the requested block in:
   - `planning/master-plan.md` (the phase/block definition)
   - Do NOT read status.md — the target block is given explicitly.

5. THINK HARD about correct scope:
   - Do not invent work beyond what the block defines.
   - Size tasks to roughly 21 hours spread across Mon/Wed/Fri sessions.
   - Enforce **the project's standing rules** as written in `CLAUDE.md` — do not assume any stack, locale-parity, or content-layout rule unless written there. Every task must leave the project's gated checks (`planning/harness.json` → `validation.checks[]` with `gates: true`) passing.
   - **Disjoint file ownership (parallel-merge safety).** A block's tasks run as parallel pipelines that merge independently, so two tasks editing the same existing file collide at merge. Decompose so each task **owns a distinct set of files**. When two tasks would touch the same file, either (a) make one `dependsOn` the other so `/sdlc-block` serializes them into different waves, or (b) restrict the shared file to **append-only** edits (the block engine union-merges files declared `additiveFiles`). Name each task's primary files in its step so the dependency analysis can see the boundaries — an undeclared overlap escalates the whole block on a merge conflict.
   - Foundational steps come first; the final step is always Validate.
   - **Use `### N. Title` heading format for every task** — sdlc-block enumerates tasks by
     this pattern (`### N.`) and will abort pre-flight on a spec that has none. Never use
     flat numbered lists (`1. **Title**`) or any other format for the task headings.

6. Create the directory `planning/phaseN-blockX/` if it does not exist, then write the spec to `planning/phaseN-blockX/tasks.md` using the Output Format below.

7. **Commit the spec.** Leave the working tree clean so a downstream `/sdlc-block` run never trips
   its clean-tree merge guard (an uncommitted `tasks.md` blocks every merge):
   ```bash
   git add planning/phaseN-blockX/
   git commit -m "chore: add spec for phaseN-blockX"
   ```
   (Use the normalized directory slug, e.g. `chore: add spec for <spec-slug>`.)

8. **Decomposition assessment.** Before reporting, evaluate each task you just wrote against the
   coarseness heuristic and recommend which (if any) warrant a `/breakdown` first. The real predictor
   is SEPARABLE STRUCTURE, not raw file count. A task is a breakdown candidate when ANY hold: it bundles
   multiple separable concerns ("implement X AND refactor Y AND add Z"), OR it spans multiple layers
   (data model + API + UI), OR it carries a large acceptance-criteria set over several independently-
   testable units, OR it touches more than `breakdown.complexityThreshold` distinct files
   (`planning/harness.json`; default 3) AND those files are HETEROGENEOUS (different shapes/roles or
   spanning more than one concern/layer). Do NOT flag on file count alone when the many files are the
   same shape serving one concern (e.g. a content path's metadata + N near-identical lesson pairs) —
   decomposition yields little there. List the flagged task numbers with a one-line reason in the report
   (the SDLC engines apply the same heuristic at run time per `breakdown.mode`, so this is the
   authoring-time preview of that decision).

9. **Pipeline recommendation.** After writing the tasks, evaluate which run command fits the block and
   report a clear recommendation with a one-line reason. Use these signals:

   - **`/sdlc-run`** — ≤3 tasks total, OR all tasks are sequential (every task depends on the previous
     one), OR the block is a single linear concern where parallel worktree isolation adds no value.
     One implement→test→review pass is sufficient.
   - **`/sdlc-block`** — ≥4 tasks AND at least 2 tasks can run in the same parallel wave (disjoint
     file ownership from step 5, no `dependsOn` between them). The orchestration and per-task worktree
     overhead pays off only when there is genuine parallelism — count the independent tasks per wave,
     not just the total task count.
   - **`/sdlc-task <N>`** — Not a strategy for running all tasks; name it only when the right move is
     one specific task in an isolated worktree (e.g. a high-risk surgical change, or resuming after a
     block failure on task N). If naming it, also say which task number and why isolation matters.

   If `breakdown.mode` is `auto` and any tasks were flagged in step 8, note that breakdown must run
   first and the pipeline recommendation applies to each resulting sub-spec, not this spec directly.

10. Report the path written and suggest the next step:
    "Spec written and committed to planning/phaseN-blockX/tasks.md. Run `/breakdown planning/phaseN-blockX/tasks.md` to decompose into atomic sub-steps."

## Context / Files to Read

- `planning/master-plan.md` (target block section only)
- `CLAUDE.md` (the project's standing rules)
- `planning/harness.json` (the project's validation checks)

## Output Format

```md
# Task Spec — Phase <N>, <Block/Project> <X>

## Goal
<one sentence, taken directly from the plan>

## Context Pointers
<which plan sections are relevant + which repo files / CLAUDE.md sections apply>

## Step-by-Step Tasks

### 1. <Foundational step>
- <bulleted actions>

### 2. <Next step>
- <bulleted actions>

<!-- ... continue; last step is always validation -->

### N. Validate
- Run the Validation Commands listed below and confirm all pass.

## Acceptance Criteria
- <specific, measurable condition>
- <specific, measurable condition>

## Validation Commands
```
<the project's validation commands — see `planning/harness.json` (`validation.checks[]`) or CLAUDE.md; one command per line, in order>
```
<!-- Add any spec-specific checks above the standard project checks. -->

## Notes
<filled in as work happens>
```

## Report

Output the path to the file created, the decomposition assessment, the pipeline recommendation, and the next-step options:
```
planning/<spec-slug>/tasks.md

Decomposition assessment:
  <"All tasks appropriately scoped." OR a list like:>
  - Task 3 — touches 6 files across model + API + UI; recommend /breakdown
  - Task 5 — bundles two separable concerns; recommend /breakdown

Pipeline recommendation:
  <one of:>
  /sdlc-run <spec-slug>          — <N> tasks, all sequential; one linear pass is sufficient
  /sdlc-block <spec-slug>        — <N> tasks, <M> can run in parallel across <W> waves; orchestration overhead worthwhile
  /sdlc-task <spec-slug> <N>     — run task <N> in isolation; <reason isolation matters here>

Next (optional — decompose first):
  /breakdown planning/<spec-slug>/tasks.md

Next (run directly):
  /<recommended-command> <spec-slug>
```
