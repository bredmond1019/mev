---
type: Spec
title: <Concept Name> — Task Spec
description: One-line summary of what this spec delivers.
---

# <Concept Name>

> Format reference for SDLC task specs. A real spec lives at `planning/<concept>/tasks.md`.
> Replace every `<...>` placeholder; keep the section headings — the pipeline reads them.

**Status:** Not started · **Last run:** _never_

## Goal

<What this spec delivers, stated as an outcome. One short paragraph. What does "done" look
like?>

## Context Pointers

- `planning/context.md` — project orientation and standing rules
- `planning/master-plan.md` — where this concept sits in the sequence
- <path/to/relevant/source — the code this spec touches>
- <link to any prior decision in planning/decisions/ that constrains this work>

## Step-by-Step Tasks

Numbered, dependency-ordered. Each task is a unit a single pipeline run can implement, test,
and review. **Use `### N. Title` heading format** — sdlc-block enumerates tasks by this
pattern and aborts pre-flight if no `### N.` headings are found.

### 1. <Foundational step>
- <what to build and where>
- Depends on: none.

### 2. <Next step>
- <what to build and where>
- Depends on: 1.

### 3. <Next step>
- <what to build and where>
- Depends on: 1.

### N. Validate
- Run the Validation Commands listed below and confirm all pass.

## Acceptance Criteria

- <Observable, checkable condition — not "works well" but "endpoint returns 200 for X">
- <Each criterion maps to something a reviewer can verify against the diff>
- All tasks ship with tests covering their core functionality.

## Validation Commands

Optional. If `planning/harness.json` exists, the pipeline runs its `validation.checks[]` and
this section is a human reference. If `harness.json` is absent, the pipeline runs the commands
listed here instead.

```bash
# <format check>
# <lint check>
# <test suite — authoritative for the verdict>
# <build>
```

## Notes

<Anything that doesn't fit above: known risks, out-of-scope items, follow-ups, links.>

## Amendment Log

Append-only. When a pipeline stage deviates from this spec (a fix, a scope adjustment, a
substitution), it records one dated line here so the spec stays a living record of how it actually
ran. Do not rewrite history — only append.

_No amendments yet._
