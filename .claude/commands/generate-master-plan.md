# Generate Master Plan — Author the project roadmap as canonical block definitions.

## Variables

$ARGUMENTS — free-text description of the project, its goal, and any planning notes from a
             planning session (paste the conversation's conclusions here). Optional `--clarify`.
             If omitted, the command reads `planning/context.md` + `CLAUDE.md` and asks the user to
             describe the goal before writing.

## Purpose

Turn a free-form planning session into a `planning/master-plan.md` whose phase/block structure
`/generate-tasks` can consume **directly**. The master plan is the roadmap source of truth: a
sequence of **block definitions** (What / Why / Build notes / Acceptance criteria), each addressable
by a parseable `phaseN-blockX` identifier. `/generate-tasks <phaseN-blockX>` later explodes one
block into a runnable `tasks.md`.

> This is the **roadmap** producer. For a single ad-hoc / experimental feature you don't yet want in
> the roadmap, use `/plan` (one standalone block) → `/generate-tasks --from`. See
> `planning/decisions/D34-adhoc-planning-seam.md`.

## Instructions

1. Run `/prime` to orient (standing rules, architecture, current state).
2. Read `planning/context.md` (why the project exists), `CLAUDE.md` (the project's actual stack +
   standing rules), and any existing `planning/master-plan.md` (you may be *revising/extending* it,
   not always writing from scratch — preserve completed phases).
3. **Clarify gate (only when enabled, OR when `$ARGUMENTS` is thin).** Read `planning/harness.json` →
   `planning.clarify`. When it is `true`, when `$ARGUMENTS` contains `--clarify`, or when the goal /
   scope / destination is genuinely underspecified, pause and ask the user **2–4 targeted clarifying
   questions** (the destination, the competence/delivery checkpoint that signals "done", the
   load-bearing architectural choices, the rough phase boundaries) before writing. Fold the answers
   in. When the gate is off and the input is already rich, proceed immediately. Strip a `--clarify`
   token before using `$ARGUMENTS` as prose.
   - **Plan-quality floor — clarify, don't fabricate (holds even when the gate is off).** The plan is
     the highest-leverage artifact; a wrong assumption here multiplies through every downstream task.
     If filling a load-bearing element (a block's Files, Acceptance criteria, scope boundary, a phase
     ordering, or a dependency) would require *inventing* a fact you cannot ground in `$ARGUMENTS`,
     `CLAUDE.md`, `planning/context.md`, the repo, or an existing `master-plan.md` — **stop and ask the
     user a targeted question** instead of writing a plausible-looking guess. The clarify gate governs
     *proactive* question rounds; this floor governs *never fabricating*. Prefer an honest "I need X to
     define block N" over a confident invention.
4. **THINK HARD about phase/block decomposition before writing:**
   - **Sequence by dependency and competence, not calendar.** Foundational, enabling work is Phase 0;
     the hardest, most-differentiating work is the last phase. `/sdlc-block` runs **phases
     sequentially and the blocks within a phase in parallel** by default; add an optional
     `- **Depends on:** Block <id>` line to a block only to override that default (e.g. to serialize
     two same-phase blocks that edit the same file).
   - A **block** is a coherent unit of work that `/generate-tasks` can turn into ~one spec (roughly a
     21-hour spread across a few sessions). Don't make blocks so large they hide separable concerns,
     nor so small they fragment one feature across many.
   - **`/generate-tasks` reads ONLY the target block's section** — not this overview, not sibling
     blocks. So every block must be **self-sufficient**: it must hand the generator the four things it
     is required to produce — concrete **files per task** (for disjoint, merge-safe decomposition),
     **observable acceptance criteria**, correct **scope boundaries**, and (if any) the **shared
     interface surface** the block leans on. Use the per-block skeleton in the Output Format (What /
     Why / Files / Interfaces / Out of scope / Acceptance criteria).
   - **Name each block's files (New vs Modified), by path.** This is load-bearing, not decoration:
     it is how `/generate-tasks` derives disjoint task ownership without guessing. Tasks that would
     share a file must be serialized (`dependsOn`) or restricted to append-only; tasks owning distinct
     files run in parallel. A block that doesn't name its files forces the generator to guess.
   - **Declare each block's Out of scope** — the explicit boundary of what belongs to a *later* block.
     If a block depends on a sibling repo or a not-yet-built project, note that prerequisite here.
   - **Distant (later-phase) blocks may be forward-looking** — author them with the full skeleton
     while the context is fresh, but say so, and expect their Files / interface lines to need
     refinement when each becomes next.
   - Do **not** bake stack/locale/deployment specifics into blocks — those live in `CLAUDE.md` +
     `planning/harness.json`. Keep block definitions about *what*, *why*, *which files*, and *bounds*.
5. Write (or revise) `planning/master-plan.md` using the Output Format below. Preserve the OKF
   frontmatter and any already-completed phases when revising.
6. **Property self-check (before reporting).** Re-read what you wrote and **revise in place** until
   every property holds, then re-check:
   - **Every block is a `### Block X — <name>` heading under a `## Phase N — <name>` heading**, so
     `/generate-tasks phaseN-blockX` can parse and locate it. No flat lists for blocks.
   - **Every block names its Files** (New vs Modified, by path), so `/generate-tasks` can derive
     ownership without guessing. A block with no named files is too thin (a forward-looking distant
     block may name them provisionally — but it must say it is provisional).
   - **Every block declares Out of scope** — at least one explicit boundary.
   - **Every block has a non-empty What, Why, and observable Acceptance criteria** that **end with the
     project's gating checks passing**. A block whose Acceptance criteria can't be judged true/false
     against a diff is too vague for `/generate-tasks`.
   - **The Quick Reference Sequence Table lists one row per block** and matches the block headings.
   - **No leftover scaffold sentinels** — no `{{TOKEN}}`, no unfilled `<...>` HTML-comment stubs, no
     empty bullets. (Legitimate `<...>` in code/prose is fine.)
7. Report the path written and the first runnable block (see Report).

## Codebase Structure

- `CLAUDE.md` — the project's actual stack, standing rules, build/test/validate commands (start here)
- `planning/context.md` — why the project exists; `planning/status.md` — current state
- `planning/harness.json` — the project's validation commands + UI-test config
- `planning/master-plan.md` — the file this command writes (the roadmap)

Read `CLAUDE.md` for the project's real stack and conventions — do not assume any framework,
language, or directory structure that isn't written there.

## Standing rules to respect

Read `CLAUDE.md` and `planning/context.md` and enforce **the project's standing rules**. CLAUDE.md is
the authority; assume no stack, locale-parity, narrative, or content-layout rule unless written
there. Universal harness rules apply: no fabricated metrics/quotes, no emoji, every block's
Acceptance criteria leave the project's gated checks (`planning/harness.json` →
`validation.checks[]`) passing. Maintain OKF frontmatter on `master-plan.md`.

## Output Format

```md
---
type: Plan
title: <Project Name> Master Plan
description: Strategic roadmap and phase specifications for <Project Name>.
---

# <Project Name> — Master Plan

*Living document. Created <DATE>.*

## The Goal, Stated Plainly
<2–3 paragraphs: what the project is, why it matters, and what "ready" means — the competence or
delivery checkpoint that signals project completion.>

## The Destination
<The named product or outcome. If commercial: the buyer, the differentiator, the through-line.>

## Architecture / Design Overview
<The key structural design: layers, an ASCII diagram with per-file/module annotations if useful, and
the load-bearing design decisions (with rationale, pointing to a decisions/ file for the full case).
Keep deployment specifics out — those live in CLAUDE.md + harness.json.>

---

## The Block Contract

`/generate-tasks` reads **only the target block's section** below — not this overview, not sibling
blocks. So every block section must be self-sufficient and hand the generator the four things it is
required to produce: concrete **files per task** (for disjoint, merge-safe decomposition),
**observable acceptance criteria**, correct **scope boundaries**, and the **shared interface surface**
it leans on. Every block uses the same skeleton:

- **What** — the scope, in implementation terms.
- **Why** — the motivation (keeps the generator from over- or under-scoping).
- **Files** — *new* vs *modified*, named by path. Load-bearing: tasks sharing a file must be
  serialized (`dependsOn`) or append-only; tasks owning distinct files run in parallel. A block that
  doesn't name its files forces the generator to guess ownership.
- **Interfaces / shared surface** *(optional — when the project has a shared API/contract layer)* —
  which shared exports/APIs/contracts the block consumes, and any *new* one it must add. Omit for
  projects with no shared layer.
- **Out of scope** — explicit boundaries; what belongs to a later block. Note any cross-repo /
  not-yet-built prerequisite here.
- **Depends on** *(optional)* — `- **Depends on:** Block <id>` (a bare `Block A` means Block A of the
  *same* phase; a fully-qualified `phase0-blockA` is also accepted). Names sibling blocks that must
  merge first. Omit it and the default order applies (see below); add it only to override that default —
  e.g. two blocks in the same phase that edit the same file must be serialized.
- **Acceptance criteria** — each a true/false condition a reviewer can check against the diff, ending
  with the project's gating checks passing.

**Default ordering — phases sequential, blocks within a phase parallel.** `/sdlc-block` runs each
phase as a wave: all blocks of Phase N run in parallel (each in its own worktree → PR), and Phase N+1
starts only after Phase N's blocks merge. A `Depends on` line refines this by adding an explicit edge,
so a dependent block waves after the block it names (use it to serialize same-phase blocks that share a
file).

Later-phase blocks may be **forward-looking** — authored with the full skeleton while the context is
fresh, but expect to refine their Files / interface lines when each becomes next (say so explicitly in
those blocks).

---

## Phase 0 — <name>

### Block A — <name>
- **What:** <scope in implementation terms — concrete enough to scope tasks>
- **Why:** <why this block, why now in the sequence>
- **Files:**
  - *New* <path> (what it holds), …
  - *Modified* <path> (what changes), …
- **Interfaces / shared surface:** <optional — shared exports/APIs this block consumes or must add>
- **Out of scope:** <explicit boundaries; what is a later block's job; any cross-repo prerequisite>
- **Depends on:** Block <id>   *(include only when a sibling in the same phase must merge first; omit this line entirely when the default phase-sequential / block-parallel order suffices)*
- **Acceptance criteria:** <observable, true/false conditions checkable against the diff; end with the
  project's gating checks passing>

### Block B — <name>
<!-- same skeleton -->

---

## Phase 1 — <name>

### Block A — <name>
<!-- same skeleton; one sub-section per block -->

---

<!-- ...continue for each phase; the last phase is the hardest, most-differentiating work.
     Distant phases' blocks carry the full skeleton but may be flagged forward-looking. -->

---

## Quick Reference Sequence Table

| Phase | Block | What | Why | Role in destination |
|---|---|---|---|---|
| 0 | A | <short> | <short> | <short> |

---

*Sequenced by dependency and competence, not calendar. When life gets in the way, pick up where you
left off.*
```

## Report

Output the path written and the next step:
```
planning/master-plan.md  (<N> phases, <M> blocks)

Blocks ready to generate:
  - phase0-blockA — <name>
  - phase0-blockB — <name>
  ...

Next (turn the first block into a runnable spec):
  /generate-tasks phase0-blockA
```
