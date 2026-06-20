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

6. Create the directory `planning/phaseN-blockX/` if it does not exist, then write the spec to `planning/phaseN-blockX/tasks.md` using the Output Format below.

7. **Commit the spec.** Leave the working tree clean so a downstream `/sdlc-block` run never trips
   its clean-tree merge guard (an uncommitted `tasks.md` blocks every merge):
   ```bash
   git add planning/phaseN-blockX/
   git commit -m "chore: add spec for phaseN-blockX"
   ```
   (Use the normalized directory slug, e.g. `chore: add spec for <spec-slug>`.)

8. Report the path written and suggest the next step:
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

Output the path to the file created and the next-step options:
```
planning/<spec-slug>/tasks.md

Next (optional — decompose into atomic sub-steps):
  /breakdown planning/<spec-slug>/tasks.md

Next (skip breakdown — implement directly):
  /implement planning/<spec-slug>/tasks.md
```
