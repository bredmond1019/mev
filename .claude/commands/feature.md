# Feature — Create a comprehensive plan to implement a new feature.

## Variables

$ARGUMENTS — description of the feature to plan.

## Instructions

1. If `$ARGUMENTS` is not provided, stop and ask the user to describe the feature.
2. **Clarify gate (only when enabled).** Read `planning/harness.json` → `planning.clarify`. When it is
   `true` **or** `$ARGUMENTS` contains `--clarify`, and the prompt is genuinely ambiguous (its scope,
   user story, target files, or success criteria could be read more than one way), pause and ask the
   user **2–4 targeted clarifying questions** before writing anything; fold the answers into the plan.
   If the prompt is already unambiguous, skip the questions and proceed even when the gate is on. When
   `planning.clarify` is absent/`false` and no `--clarify` flag is present, skip this step entirely and
   behave exactly as before (write immediately). Strip a `--clarify` token from the prompt before using
   it as the `prompt:` metadata value.
3. THINK HARD about the feature's scope, design, and how it fits the existing site before writing anything.
4. Research the codebase: read `CLAUDE.md` and any relevant docs the project keeps, then any files directly relevant to the feature.
5. Create a plan using the Plan Format below.
6. Choose a short descriptive slug (e.g. `add-rss-feed`, `add-search`, `add-newsletter-signup`).
7. Create the directory `planning/feature-{descriptive-name}/` if it does not exist, then save the plan to `planning/feature-{descriptive-name}/tasks.md`.
8. **Property self-check.** Before reporting, re-read the spec and confirm every required property
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
unless written there. Universal harness rules still apply: no fabricated metrics/quotes (verify model
ids / package names via the `claude-api` skill, not memory), no emoji, every change ships with tests.

## Plan Format

```md
# Feature: <feature name>

## Metadata
prompt: `{$ARGUMENTS}`
status: Not started
last-run: never

## Feature Description
<describe the feature in detail — what it does, why it's needed, who/what benefits>

## User Story
As a <type of visitor or site component>
I want to <action or goal>
So that <benefit or outcome>

## Problem Statement
<the specific problem or gap this feature addresses>

## Solution Statement
<the proposed approach and why it fits the project's existing patterns>

## Relevant Files
<list files relevant to the feature with bullet points explaining why each is needed>

### New Files
<list all new files to be created with a one-line description of each — include any companion files the change requires>

## Implementation Plan

### Phase 1: Foundation
<types, shared utilities, service-layer changes, content scaffolding — anything that must land first>

### Phase 2: Core Implementation
<the components, routes, service logic, and main business logic>

### Phase 3: Integration
<wiring into the rest of the system, API/routes, content hookup, end-to-end behavior>

## Step by Step Tasks
IMPORTANT: Execute every step in order, top to bottom.
Include writing tests throughout — do not leave them to the end.

### 1. <First Task Name>
- <specific action>
- <specific action>

### 2. <Second Task Name>
- <specific action>

### N. Validate
- Run the Validation Commands listed below and confirm all pass.

## Testing Strategy

### Unit Tests
<list the tests to write and what each should cover>

### Edge Cases
<list edge cases that must be tested — empty input, missing data, malformed input, etc.>

## Acceptance Criteria
<list specific, measurable conditions that must be true for the feature to be done>

## Validation Commands
```
<the project's validation commands — see `planning/harness.json` (`validation.checks[]`) or CLAUDE.md; one command per line, in order>
```
<add any feature-specific end-to-end or integration checks above the standard project checks>

## Notes
<dependencies, new packages needed, deferrals, future considerations, known constraints>

## Amendment Log
<!-- Append-only. Pipeline stages append one dated line here when they deviate from the spec. -->
_No amendments yet._
```

## Report

Output the path to the plan file created and the next-step options:
```
planning/feature-{name}/tasks.md

Next (optional — decompose into atomic sub-steps):
  /breakdown planning/feature-{name}/tasks.md

Next (skip breakdown — implement directly):
  /implement planning/feature-{name}/tasks.md
```
