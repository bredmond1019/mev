# Chore — Plan a maintenance or housekeeping task.

## Variables

$ARGUMENTS — description of the chore to plan.

## Instructions

1. If `$ARGUMENTS` is not provided, stop and ask the user to describe the chore.
2. Research the codebase: read `CLAUDE.md`, then any files directly relevant to the chore.
3. Create a plan using the Plan Format below.
4. Choose a short descriptive slug for the chore (e.g. `remove-k8s-secret`, `fix-devin-typos`, `update-stale-handles`).
5. Create the directory `planning/chore-{descriptive-name}/` if it does not exist, then save the plan to `planning/chore-{descriptive-name}/tasks.md`.
6. Return only the path to the file created.

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
# Chore: <chore name>

## Metadata
prompt: `{$ARGUMENTS}`

## Chore Description
<describe the chore in detail — what it is, why it matters, any known constraints>

## Relevant Files
<list files relevant to the chore with bullet points explaining why each is needed>

### New Files
<list any new files that will be created, if applicable>

## Step by Step Tasks
IMPORTANT: Execute every step in order, top to bottom.

### 1. <First Task Name>
- <specific action>
- <specific action>

### 2. <Second Task Name>
- <specific action>

### N. Validate
- Run the Validation Commands listed below and confirm all pass.

## Validation Commands
```
<the project's validation commands — see `planning/harness.json` (`validation.checks[]`) or CLAUDE.md; one command per line, in order>
```
<add any chore-specific checks above the standard project checks>

## Notes
<optional context, edge cases, or gotchas>
```

## Report

Output the path to the plan file created and the next-step options:
```
planning/chore-{name}/tasks.md

Next (optional — decompose into atomic sub-steps):
  /breakdown planning/chore-{name}/tasks.md

Next (skip breakdown — implement directly):
  /implement planning/chore-{name}/tasks.md
```
