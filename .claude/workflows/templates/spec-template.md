---
type: Spec
title: <Concept Name> — Task Spec
description: One-line summary of what this spec delivers.
---

# <Concept Name>

> Format reference for SDLC task specs. A real spec lives at `planning/<concept>/tasks.md`.
> Replace every `<...>` placeholder; keep the section headings — the pipeline reads them.

## Goal

<What this spec delivers, stated as an outcome. One short paragraph. What does "done" look
like?>

## Context Pointers

- `planning/context.md` — project orientation and standing rules
- `planning/master-plan.md` — where this concept sits in the sequence
- <path/to/relevant/source — the code this spec touches>
- <link to any prior decision in planning/decisions/ that constrains this work>

## Tasks

Numbered, dependency-ordered. Each task is a unit a single pipeline run can implement, test,
and review. Note dependencies so parallel waves can be computed.

1. **<Task 1 — short title>** — <what to build and where>. Depends on: none.
2. **<Task 2 — short title>** — <what to build and where>. Depends on: 1.
3. **<Task 3 — short title>** — <what to build and where>. Depends on: 1.

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
