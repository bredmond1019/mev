# Plan — Create a plan for a task, scaled to its complexity.

## Purpose

Plan one ad-hoc / experimental task or feature from a free-text description. The resulting `plan.md`
serves two downstream routes:
- **Direct:** `/breakdown` or `/implement` it as-is (the quick path).
- **Rigorous:** treat it as a **standalone block definition** and run `/generate-tasks --from
  planning/plan-<slug>/plan.md` to decompose it into a `tasks.md` with full disjoint-ownership
  analysis, an `execution-plan.json`, and a pipeline recommendation — then run it (e.g. via
  `/sdlc-flow`) on a feature branch. This is how you try an experimental feature **without** putting
  it in `master-plan.md` first. See `planning/decisions/D34-adhoc-planning-seam.md`. (For the roadmap
  itself, use `/generate-master-plan`.)

## Variables

$ARGUMENTS — description of the task to plan.

## Instructions

1. If `$ARGUMENTS` is not provided, stop and ask the user to describe the task.
2. **Clarify gate (only when enabled).** Read `planning/harness.json` → `planning.clarify`. When it is
   `true` **or** `$ARGUMENTS` contains `--clarify`, and the prompt is genuinely ambiguous (its scope,
   target files, or success criteria could be read more than one way), pause and ask the user **2–4
   targeted clarifying questions** before writing anything; fold the answers into the plan. If the
   prompt is already unambiguous, skip the questions and proceed even when the gate is on. When
   `planning.clarify` is absent/`false` and no `--clarify` flag is present, skip this step entirely and
   behave exactly as before (write immediately). Strip a `--clarify` token from the prompt before using
   it as the `prompt:` metadata value.
   - **Plan-quality floor — clarify, don't fabricate (holds even when the gate is off).** If filling a
     load-bearing element (the Relevant Files / New Files, a `### N.` task's concrete files, Acceptance
     Criteria, or a dependency) would require *inventing* a fact you cannot ground in `$ARGUMENTS`,
     `CLAUDE.md`, `planning/context.md`, or the repo — **stop and ask the user** rather than write a
     plausible-looking guess. The gate governs proactive question rounds; this floor governs never
     fabricating. An honest "I need X" beats a confident invention.
3. THINK HARD about task type and complexity before writing anything:
   - `task_type`: `chore` | `feature` | `refactor` | `fix` | `enhancement` | `content`
   - `complexity`: `simple` | `medium` | `complex`
   - Simple tasks (chores, targeted fixes, single-post content edits): focus on specific changes and validation.
   - Complex tasks (features, refactors, multi-page content arcs): include design rationale, implementation phases, and testing strategy.
4. Research the codebase: read `CLAUDE.md`, then files directly relevant to the task.
5. Create a plan using the Plan Format below, omitting sections marked as conditional when they don't apply.
6. Choose a short descriptive slug (e.g. `fix-claude-sdk-package-name`, `refactor-content-loader`, `add-build-cache`).
7. Create the directory `planning/plan-{descriptive-name}/` if it does not exist, then save the plan to `planning/plan-{descriptive-name}/plan.md`.
8. **Property self-check.** Before reporting, re-read the plan and confirm every required property
   holds; **revise in place** if any fails, then re-check:
   - **Every `### N.` task names ≥1 concrete file** it creates or modifies (the final Validate step is
     exempt).
   - **Acceptance Criteria are non-empty and observable** — each can be judged true/false.
   - **Validation Commands are present** (or `planning/harness.json` → `validation.checks[]` supplies
     them as the fallback).
   - **No leftover template sentinels** — no `{{TOKEN}}`, unfilled `<placeholder>`-style angle stubs,
     or empty bullets. Do **not** treat legitimate `<...>` in code/prose or a bare `TODO`/`TBD` inside
     authored content as a sentinel.
9. Return only the path to the file created.

## Codebase Structure

- `CLAUDE.md` — standing rules, the SDLC pipeline, build/test/validate commands (start here)
- `planning/context.md` — why the project exists + audit findings; `planning/status.md` — progress
- `planning/harness.json` — the project's validation commands + UI-test config
- `planning/` — task specs and plan files (one concept folder per task)

Read `CLAUDE.md` for the project's actual stack, directory layout, and conventions — do not assume
any framework, language, or directory structure that isn't written there.

## Standing rules to respect

Read `CLAUDE.md` and `planning/context.md` — internalize and enforce **the project's standing rules**.
CLAUDE.md is the authority; do not assume any stack, locale-parity, narrative, or content-layout rule
unless written there. Universal harness rules still apply: no fabricated metrics/quotes, no emoji,
every change ships with tests.

## Plan Format

```md
# Plan: <task name>

## Metadata
prompt: `{$ARGUMENTS}`
task_type: <chore|feature|refactor|fix|enhancement|content>
complexity: <simple|medium|complex>
status: Not started
last-run: never

## Task Description
<describe the task in detail based on the prompt>

## Objective
<one sentence: what will be true when this plan is fully executed>

<!-- Include for feature/refactor/complex tasks: -->
## Problem Statement
<the specific problem or opportunity this task addresses>

## Solution Approach
<the proposed solution and why it fits the project's existing patterns>
<!-- end conditional -->

## Relevant Files
<list files relevant to the task with bullet points explaining why each is needed>

### New Files
<list any new files to be created, if applicable — include any companion files the change requires>

<!-- Include for medium/complex tasks: -->
## Implementation Phases
### Phase 1: Foundation
<any foundational work that must land first>

### Phase 2: Core Implementation
<the main body of work>

### Phase 3: Integration & Validation
<integration with existing pages/services, tests, final checks>
<!-- end conditional -->

## Step by Step Tasks
IMPORTANT: Execute every step in order, top to bottom.

### 1. <First Task Name>
- <specific action>
- <specific action>

### 2. <Second Task Name>
- <specific action>

### N. Validate
- Run the Validation Commands listed below and confirm all pass.

<!-- Include for feature/complex tasks: -->
## Testing Strategy
<tests needed; edge cases to cover; any integration test requirements>
<!-- end conditional -->

## Acceptance Criteria
<list specific, measurable conditions that must be true for this task to be done>

## Validation Commands
```
<the project's validation commands — see `planning/harness.json` (`validation.checks[]`) or CLAUDE.md; one command per line, in order>
```
<add any task-specific checks above the standard project checks>

## Notes
<optional: dependencies, new packages needed, deferrals, constraints, follow-ups>

## Amendment Log
<!-- Append-only. Pipeline stages append one dated line here when they deviate from the plan. -->
_No amendments yet._
```

## Report

Output the path to the plan file created and the next-step options:
```
planning/plan-{name}/plan.md

Next (optional — decompose into atomic sub-steps):
  /breakdown planning/plan-{name}/plan.md

Next (skip breakdown — implement directly):
  /implement planning/plan-{name}/plan.md

Next (rigorous — decompose into a runnable spec, then run on a branch):
  /generate-tasks --from planning/plan-{name}/plan.md
  /sdlc-flow plan-{name}
```
