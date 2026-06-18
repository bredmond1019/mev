# Plan — Create a plan for a task, scaled to its complexity.

## Variables

$ARGUMENTS — description of the task to plan.

## Instructions

1. If `$ARGUMENTS` is not provided, stop and ask the user to describe the task.
2. THINK HARD about task type and complexity before writing anything:
   - `task_type`: `chore` | `feature` | `refactor` | `fix` | `enhancement` | `content`
   - `complexity`: `simple` | `medium` | `complex`
   - Simple tasks (chores, targeted fixes, single-post content edits): focus on specific changes and validation.
   - Complex tasks (features, refactors, multi-page content arcs): include design rationale, implementation phases, and testing strategy.
3. Research the codebase: read `CLAUDE.md`, then files directly relevant to the task.
4. Create a plan using the Plan Format below, omitting sections marked as conditional when they don't apply.
5. Choose a short descriptive slug (e.g. `fix-claude-sdk-package-name`, `refactor-content-loader`, `add-build-cache`).
6. Create the directory `planning/plan-{descriptive-name}/` if it does not exist, then save the plan to `planning/plan-{descriptive-name}/plan.md`.
7. Return only the path to the file created.

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
```

## Report

Output the path to the plan file created and the next-step options:
```
planning/plan-{name}/plan.md

Next (optional — decompose into atomic sub-steps):
  /breakdown planning/plan-{name}/plan.md

Next (skip breakdown — implement directly):
  /implement planning/plan-{name}/plan.md
```
