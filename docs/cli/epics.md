---
type: Reference
title: mev CLI — epic and operator-gate commands
description: The commands that move an initiative through its lifecycle and clear the gates that block work on a human decision.
doc_id: cli-epics
layer: [factory]
project: mev
status: active
keywords: [epics, operator gate, approval, decision gate, slugs]
related: [cli-reference, cli-state, architecture]
---

# mev CLI — epic and operator-gate commands

Part of the [CLI reference](../cli.md).

## What this page is for

Two things in this system block work for reasons that are not code.

An **epic** is a named initiative that owns a set of blocks across repos. Parking one should park
its blocks; finishing one should not leave stragglers behind. The four `*-epic` commands keep the
registry and the blocks agreeing in both directions.

An **operator gate** is a `depends_on` edge saying *only a human can do this next* — make a
decision, supply a credential, look at something. Work behind it cannot start until the gate is
closed, and closing it requires naming the artifact that proves the work happened. That refusal is
the feature, not an obstacle.

| Command | Does |
|---|---|
| [`defer-epic` / `resume-epic` / `complete-epic` / `sync-epics`](#defer-epic-slug---write-path--resume-epic-slug---write-path--complete-epic-slug---write-path--sync-epics---write-path) | Park, un-park, finish, or reconcile an initiative |
| [`close-operator-gate`](#close-operator-gate-slug---exit-verified-path---write) | Clear a human-work gate fleet-wide, once its exit artifact exists |
| [`approve` / `reject`](#approve-slug---digest-digest-path---write--reject-slug-path---write) | Settle a yes/no decision gate on a fixed payload |
| [`normalize-op-slugs`](#normalize-op-slugs---write-path) | Fix stuttering operator/approval slugs fleet-wide |

## Quickstart

Run these in a **terminal**. All of them write fleet-wide under `--write`; all dry-run by default.

```bash
# Park an initiative and cascade `deferred` to its blocks
mev defer-epic brain-quality --write
mev resume-epic brain-quality --write

# Registry and blocks disagree? Reconcile both directions
mev sync-epics --write

# Close a human-work gate — REFUSES unless you assert the exit artifact exists
mev close-operator-gate operator-rate-card --exit-verified --write

# Settle a decision gate on a fixed payload
mev approve some-slug --digest sha256:abc123 --write
```

**`close-operator-gate` refuses without `--exit-verified`.** That flag is you asserting the gate's
named exit artifact actually exists. It is not a formality — the gate exists precisely because
nothing automatic can check that.

## Commands

### `defer-epic <slug> [--write] [path]` · `resume-epic <slug> [--write] [path]` · `complete-epic <slug> [--write] [path]` · `sync-epics [--write] [path]`

Park and un-park a whole initiative, keeping the HQ `epics[]` registry status and
its member blocks' authored statuses in agreement.

An epic is "parked" when its registry `status` is `paused` **and** its unfinished
member blocks are `deferred`. Those two can drift — a `paused` epic whose blocks
are still `open` keeps flooding `focus.next` even though you consider the
initiative shelved. These commands move both together.

| Command | Registry | Member blocks |
|---|---|---|
| `defer-epic <slug>` | → `paused` | `open` → `deferred` |
| `resume-epic <slug>` | → `active` | `deferred` → `open` |
| `complete-epic <slug>` | → `complete` | untouched |
| `sync-epics` | fully-deferred epics → `paused` | stragglers in a `paused` epic → `deferred` |

**`complete-epic` is the odd one out: it never cascades.** `defer-epic` and
`resume-epic` move member blocks along with the registry; `complete-epic` sets
*only* the registry epic's status to `complete` and touches zero member blocks.
It is an **operator declaration** that an initiative is finished, not something
mev infers — `W_STATE_EPIC_ALL_CLOSED` (all members closed) stays warn-only by
design, precisely because the last block closing is not the same as the goal
being met. A `complete` epic drops off the board entirely. There is no
`reopen-epic`; if that turns out to be wrong, undo it by hand in the registry.

**`in_progress` blocks are never touched**, in either direction. Parking work you
are mid-block on is far more likely to be a mistake than an intent, so it is left
alone and reported as `W_EPIC_SKIPPED_IN_PROGRESS`. `closed` blocks are likewise
never reopened.

**`sync-epics` never un-defers anything.** An `active` epic with *some* deferred
blocks is a perfectly normal state (you parked two of nine). Un-parking is always
explicit, via `resume-epic`.

**Dry-run by default**, exactly like `emit-state`: without `--write` the proposed
edits print as `W_EMIT_DRY_RUN` and nothing is touched. A successful `--write`
additionally runs `emit-state --write`, so `focus`, the boards and the rollups are
regenerated in the same invocation instead of being left drifted.

**`--write` takes the same advisory lock `emit-state --write` and
`set-block-status --write` take**, at `<root>/.mev-emit.lock`, before any file is
touched — `defer-epic`, `resume-epic`, `complete-epic`, and `sync-epics` all
share one dispatch function, so one lock acquisition covers all four. If another live process
already holds it, the command fails with `E_EMIT_LOCK_HELD` (naming the holder's
pid) and writes nothing; a lockfile whose owning process is no longer alive is
reclaimed automatically instead of blocking forever. Dry-run (no `--write`) never
takes the lock and is unaffected by contention. **The same quiesce-lease check
`emit-state --write` runs also runs here, before the lock is taken** — a sibling
lane's exclusive lease refuses with `E_QUIESCE_LEASE_HELD` instead; pass `--agent`
to self-exempt a lease this same caller holds. See [Quiesce lease on
`--write`](state.md#quiesce-lease-on---write---agent---lock-dir).

> **These, plus [`set-block-status`](#set-block-status-repoid-status---write-path),
> are the only commands that write *authored* state.** Everything else mev writes is
> derived. The cascade lives behind an explicit command precisely so `emit-state`
> stays safe to run unattended — see `src/brain/epics.rs` for the full rationale.

```bash
# What would parking the TUI initiative change?
mev defer-epic bastion-tui

# Park it (and regenerate every derived view)
mev defer-epic bastion-tui --write

# Bring it back
mev resume-epic bastion-tui --write

# You deferred blocks by hand; make the registry agree
mev sync-epics --write

# Declare the initiative finished (registry only — no member block is touched)
mev complete-epic bastion-tui --write
```

Exit codes: `0` planned/applied successfully · `1` unknown epic slug
(`E_EPIC_UNKNOWN`), no HQ registry (`E_EPIC_NO_REGISTRY`), an unreadable
state.json (`E_EPIC_INCOMPLETE_CORPUS` on `--write`), the advisory lock already
held (`E_EMIT_LOCK_HELD` on `--write`), a sibling lane's quiesce lease
(`E_QUIESCE_LEASE_HELD` on `--write` — see [Quiesce lease on
`--write`](state.md#quiesce-lease-on---write---agent---lock-dir)), or a write failure.

---

### `close-operator-gate <slug> --exit-verified [path] [--write]`

Removes every `Operator` `depends_on` edge carrying `slug`, fleet-wide, under the
same advisory lock `emit-state --write` takes. This is a **verified-or-refused**
command, not a dry-run/`--write`-shaped planner like the epic commands: it refuses
outright, before any file is read, unless `--exit-verified` is passed — passing
the flag is the caller asserting the gate's exit condition has actually been
checked, not a formality. An unknown slug (no loaded file has a matching edge)
is also refused. A successful `--write` re-runs `emit-state --write` so `focus`
and the boards drop the closed gate in the same invocation.

```bash
mev close-operator-gate deploy-approval-2 --exit-verified ~/Dev/agentic-portfolio --write
```

| Diagnostic | Cause |
|---|---|
| `E_OPERATOR_GATE_NOT_VERIFIED` | `--exit-verified` was not passed |
| `E_OPERATOR_GATE_UNKNOWN` | no loaded `state.json` has an `Operator` edge matching `slug` |
| `E_EMIT_LOCK_HELD` | another mev write holds the brain-root advisory lock |
| `E_QUIESCE_LEASE_HELD` | a sibling lane's exclusive lease declares a quiet window over this write; `close-operator-gate` keeps its verified-or-refused shape — this refusal is already that vocabulary, no dry-run was added. See [Quiesce lease on `--write`](state.md#quiesce-lease-on---write---agent---lock-dir). |

Exit codes: `0` applied · `1` refused or a write failure.

---

### `normalize-op-slugs [--write] [path]`

Renames every **stuttering** operator/approval slug (D76) fleet-wide, in one
atomic pass per slug. A slug carrying a redundant `operator-` prefix (e.g.
`operator-mac-mini-visit`) stutters when rendered as `OP.<slug>` (see the
rendering note below) — this is the fix. It finds every stuttering slug
anywhere in the loaded corpus, groups **all** `operator`/`approval`
`depends_on` edges carrying that exact slug — across every file, every repo —
and renames every one of them to `okf_core::normalize_op_slug`'s target in one
pass. One slug can gate several blocks across several repos; renaming some of
its edges but not others would split one shared gate into two, so a slug is
always renamed everywhere at once or not at all.

**Collision detection runs before any write.** The full rename plan (every
distinct stuttering slug found, mapped to its normalized target) is computed
first. If two distinct slugs would normalize to the same target — including a
stuttering slug colliding with an already-existing non-stuttering slug — the
**entire run aborts with no writes at all**, even for the renames in the same
corpus that did not collide. Silently merging two distinct gates into one
shared identity is worse than leaving both stuttering.

**Dry-run by default**, exactly like the epic commands and `set-block-status`:
without `--write` the plan prints as an `I_NORMALIZE_OP_SLUG_PLAN` diagnostic
per distinct rename (old slug, new slug, edge count, repos touched) plus a
`W_EMIT_DRY_RUN` note per file, and not a byte is touched. A successful
`--write` takes the same advisory lock `emit-state --write` takes, refuses to
run against an incomplete corpus, and then runs `emit-state --write` so
`focus`, the boards and the rollups — including the `OP.<slug>` rendering
below — reflect the renamed slugs in the same invocation. Refused the same way
as `set-block-status` when run from inside a linked git worktree.

```bash
# What would normalizing every stuttering slug change? (dry run — writes nothing)
mev normalize-op-slugs ~/Dev/agentic-portfolio

# Apply it fleet-wide, and regenerate every derived view
mev normalize-op-slugs ~/Dev/agentic-portfolio --write
```

| Diagnostic | Cause |
|---|---|
| `E_NORMALIZE_OP_SLUG_COLLISION` | two distinct slugs would normalize to the same target — aborts the whole run, nothing written |
| `E_EMIT_INCOMPLETE_CORPUS` | `--write` attempted while at least one `state.json` failed to load |
| `E_EMIT_LOCK_HELD` | another mev write holds the brain-root advisory lock |
| `E_QUIESCE_LEASE_HELD` | a sibling lane's exclusive lease declares a quiet window; do not retry — see [Quiesce lease on `--write`](state.md#quiesce-lease-on---write---agent---lock-dir) |

Exit codes: `0` planned (dry-run) or applied cleanly · `1` a collision, a
write failure, `E_EMIT_LOCK_HELD`, `E_QUIESCE_LEASE_HELD`, or a linked-worktree refusal.

**Rendering note (D76).** Operator and approval `depends_on` edges now render
as `OP.<slug>` everywhere mev prints them (boards, `frontier`, `carryover`,
etc.) instead of the old hand-rolled `operator:<slug>`/`approval:<slug>`
prefixes — both edge kinds share one flat `OP.<slug>` identity; surrounding
context (an `exit`/`start` pair vs. a `decision`) already disambiguates which
kind it is. Rendering is faithful, not normalizing: a stuttering slug still
renders stuttered (`OP.operator-mac-mini-visit`) until `normalize-op-slugs`
renames it. `validate-brain --state` also warns `W_STATE_OP_SLUG_STUTTER` on
any stuttering slug it finds — a warning only, it never flips the exit code.
See `docs/decisions/D76-operator-sessions-get-a-flat-op-id.md` (in the company
brain) for the full rationale.

---

### `approve <slug> --digest <digest> [path] [--write]` · `reject <slug> [path] [--write]`

Remove every `Approval` `depends_on` edge carrying `slug`, fleet-wide, under the
same advisory lock. `approve` additionally requires `--digest` to match every
matching edge's stored `digest`; a mismatch on *any* matching edge refuses the
whole call and changes nothing (`E_APPROVAL_DIGEST_MISMATCH`) rather than
silently re-queuing the block — a shared slug is meant to carry one reviewed
payload. `reject` takes no digest and always clears matching edges; the
rejection is recorded via a non-suppressible `I_EMIT_WROTE` diagnostic (the same
pattern `close-operator-gate` uses), not a separate log file. Both re-run
`emit-state --write` on a successful `--write`.

```bash
mev approve ship-decision-1 --digest sha256:9f2c... ~/Dev/agentic-portfolio --write
mev reject ship-decision-1 ~/Dev/agentic-portfolio --write
```

| Diagnostic | Cause |
|---|---|
| `E_APPROVAL_DIGEST_MISMATCH` | (`approve` only) `--digest` does not match a matching edge's stored digest |
| `E_OPERATOR_GATE_UNKNOWN` | no loaded `state.json` has an `Approval` edge matching `slug` |
| `E_EMIT_LOCK_HELD` | another mev write holds the brain-root advisory lock |
| `E_QUIESCE_LEASE_HELD` | a sibling lane's exclusive lease declares a quiet window; do not retry — see [Quiesce lease on `--write`](state.md#quiesce-lease-on---write---agent---lock-dir) |

Exit codes: `0` applied · `1` refused or a write failure.

---

