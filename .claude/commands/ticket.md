# Ticket — Plan a small behavior-change with observable Acceptance Criteria.

## Variables

$ARGUMENTS — description of the bug fix, enhancement, or small behavior-change to implement.

## Purpose

Plan one small, well-scoped behavior-change — a bug fix or targeted enhancement that requires
new or modified tests. The output is a single-block `tasks.md` with `### N.` tasks, explicit
Acceptance Criteria, and a Testing Strategy, feeding directly into lean `/sdlc-task`.

> **Distinct from `/chore`:** chores are maintenance (no behavior change, tests incidental).
> Tickets are behavior-changing (tests required, AC is non-negotiable).
> For multi-block work, use `/plan` → `/sdlc-block` instead.

## Instructions

1. If `$ARGUMENTS` is not provided, stop and ask the user to describe the bug or change.
2. **Plan-quality floor — clarify, don't fabricate.** If filling a load-bearing element (which
   files to change, what the observable correct behavior is, or an Acceptance Criterion) would
   require *inventing* a fact you cannot ground in `$ARGUMENTS`, `CLAUDE.md`, `planning/context.md`,
   or the repo — **stop and ask the user a targeted question** rather than write a plausible-looking
   guess. An honest "I need X to write the AC" beats a confident invention.
3. Research the codebase: read `CLAUDE.md`, `planning/context.md`, then the files directly
   relevant to the change.
4. THINK HARD about scope before writing:
   - A ticket is a **single coherent unit** — one logical change, one set of tests.
   - If the fix touches more than 3–4 files or needs its own sub-phases, it belongs in `/plan`.
   - Every `### N.` task must name ≥1 concrete file it creates or modifies (the Validate task
     is exempt).
5. Choose a short descriptive slug (e.g. `fix-null-deref`, `add-rate-limit`, `patch-auth-refresh`).
6. Create `planning/ticket-{slug}/` if it does not exist, then write the spec to
   `planning/ticket-{slug}/tasks.md` using the Plan Format below.
7. **Property self-check.** Before reporting, re-read the spec and **revise in place** until every
   property holds, then re-check:
   - **Every `### N.` task names ≥1 concrete file** it creates or modifies (Validate is exempt).
   - **Acceptance Criteria are non-empty and observable** — each can be judged true/false.
   - **Testing Strategy is non-empty** — names the test file(s) and what each must cover.
   - **Validation Commands are present** (or `planning/harness.json` → `validation.checks[]`
     supplies them as the fallback).
   - **No leftover template sentinels** — no `{{TOKEN}}`, unfilled `<placeholder>`-style angle
     stubs, or empty bullets. Legitimate `<...>` in code/prose is fine.
8. Report the path and next step.

## Codebase Structure

- `CLAUDE.md` — standing rules, stack, build/test/validate commands (start here)
- `planning/context.md` — why the project exists; `planning/status.md` — current state
- `planning/harness.json` — validation commands + UI-test config
- `planning/` — task specs (one concept folder per task)

Read `CLAUDE.md` for the project's actual stack and conventions — do not assume any framework,
language, or directory structure that isn't written there.

## Standing rules to respect

Read `CLAUDE.md` and `planning/context.md` and enforce **the project's standing rules**. CLAUDE.md
is the authority. Universal harness rules apply: no fabricated metrics/quotes, no emoji, every
ticket ships with tests.

## Plan Format

```md
# Ticket: <change name>

## Metadata
prompt: `{$ARGUMENTS}`
status: Not started
last-run: never

## Description
<what is broken or missing and what the correct behavior should be; one concise paragraph>

## Relevant Files
<files to change, with a one-line note on why each is needed>

### New Files
<new files to create, if any — test files go here when they don't exist yet>

## Step by Step Tasks
IMPORTANT: Execute every step in order, top to bottom.

### 1. <First Task Name>
- Files: <file(s) this task touches>
- <specific action>

### 2. <Second Task Name>
- Files: <file(s) this task touches>
- <specific action>

### N. Validate
- Run the Validation Commands listed below and confirm all pass.

## Testing Strategy
<which test file(s) cover this change; what behavior each test must assert; any edge cases>

## Acceptance Criteria
<list specific, observable conditions that must be true for this ticket to be done>

## Validation Commands
<the project's validation commands — see `planning/harness.json` or CLAUDE.md; one per line>

## Notes
<optional: constraints, follow-ups, known edge cases not covered by this ticket>

## Amendment Log
<!-- Append-only. Pipeline stages append one dated line here when they deviate from the plan. -->
_No amendments yet._
```

## Report

Output the path and next step:
```
planning/ticket-{slug}/tasks.md

Next (implement + test loop):
  /sdlc-task ticket-{slug}
```
