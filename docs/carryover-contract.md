---
type: Reference
title: "Carryover Triage Ranking Contract"
description: "The versioned, canonical contract for the four-lane carryover triage ranking mev produces — the public Rust API bastion calls and the wire shape it projects."
doc_id: carryover-contract
layer: [brain, console]
project: mev
status: active
keywords: [carryover, triage, ranking, attention, effective-priority, contract, blocks]
related: [brain:carryover-improvements-plan, brain:state-json-schema, cli-reference]
---

# Carryover Triage Ranking Contract

## What this page is for

This is a **contract between repos**, not a guide. `mev` derives the carryover triage ranking;
`bastion` renders it. This page pins the API and wire shape so the two cannot drift apart, and it
is versioned for that reason.

If you want to *use* the carryover commands rather than consume their output, you want
[the carryover CLI page](cli/carryover.md) instead.

## Quickstart

Run these in a **terminal** to see the shape this contract describes:

```bash
mev carryover --json | head -40     # the wire shape bastion consumes
mev carryover                       # the same data, human-readable
```

**Changing anything below is a breaking change.** Bump the contract version, note it in the
changelog, and re-pin the consumer copy in `core/bastion/docs/`.

**Contract Version: 1.0.0**

This is the **single source of truth** for how a `planning/state.json` `carryover[]` array is
turned into a ranked, four-lane triage board. mev **owns** this document — it derives the ranking.
Consumers (today: `bastion`, via `BA.ticket.carryover-triage-dto`) reference and *pin* it; they
never fork it or re-derive the ranking themselves. When any shape here changes, bump the version
and add a changelog row (see [Versioning](#versioning)).

This is a **type contract plus a wire shape**, not prose. bastion depends on mev **by path**
(`core/bastion/Cargo.toml:36`) and already delegates `carryover_stale_age` — its doc comment at
`core/bastion/src/serve/handlers/attention.rs:140-141` states it *"never reimplements that
predicate."* The ranking in this document extends the same discipline: mev exposes a **public
function**, bastion **calls it and projects the result**.

---

## 1. Exported surface

All items below are re-exported from `mev`'s crate root (`src/lib.rs`), so a path-dependent
consumer reaches them as `mev::rank_carryover`, `mev::CarryoverRanking`, etc.

### `rank_carryover` — the entry point

```rust
pub fn rank_carryover(
    entries: &[CarryoverVerdict],
    block_priorities: &HashMap<String, u8>,
    block_status: &HashMap<String, Option<String>>,
) -> Vec<CarryoverRanking>
```

- `entries` — every `carryover[]` verdict to rank (typically a full fleet-wide sweep's
  `CarryoverReport.entries`).
- `block_priorities` — each **block** node's already-computed effective priority, keyed
  `"{repo}:{id}"` (the same map `state::effective_priorities` produces). Treated as terminal — this
  function never recomputes or changes a block's effective priority.
- `block_status` — each block node's authored status, keyed `"{repo}:{id}"`, `None` when the target
  is unresolvable. Used only to decide whether a `blocks[]` edge is met.
- Returns every entry as a [`CarryoverRanking`](#3-carryoverranking), fully sorted per
  [§4](#4-the-four-lanes). **The caller must never re-sort or re-derive lane/priority** — the
  returned order and the returned `lane`/`effective_priority` fields are the contract.

### `TriageLane`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TriageLane { Blocking, Hot, Aging, Standing }
```

Serializes as `"blocking" | "hot" | "aging" | "standing"`. See [§4](#4-the-four-lanes) for
membership and ordering.

### `CarryoverRanking`

See [§3](#3-carryoverranking) below — the per-entry wire shape.

### `CarryoverVerdict` (input type, already public)

The evaluated verdict for one `carryover[]` entry, produced by `evaluate_carryover`
(`MV.ticket.carryover-field-validation` / `MV.ticket.clears-when-evaluation`). Carries `repo`,
`slug`, `kind`, `text`, `clears_when`, `created`, `age_days: Option<i64>`, `stale: bool`,
`lane: CarryoverLane` (the cleared/actionable/not-evaluable verdict — distinct from `TriageLane`),
`refs`, `reason`, `priority: Option<u8>`, `finding_id: Option<String>`, and
`blocks: Vec<BlockedBy>`. `rank_carryover` consumes this type directly; a consumer assembling its
own `CarryoverVerdict` values must populate `priority`, `finding_id`, and `blocks` from the source
`Carryover` item verbatim — these are **never reconciled or averaged** across repos (see §5).

### `FindingCluster` (input type, already public)

Groups every entry sharing an authored `finding_id` (`MV.ticket.carryover-dedup-clusters`):
`finding_id: String`, `members: Vec<ClusterMember>`, `repos: Vec<String>` (sorted, deduplicated),
`single_repo: bool` (the cross-repo-typo guard — `true` means the id links nothing across repos).
Not consumed by `rank_carryover` directly; documented here because it is the other structured
output of the same `carryover` sweep and a consumer projecting the full board typically reads both.

---

## 2. Board membership no longer gates on staleness alone

**This is the behavioral change a consumer must account for.** Before this contract, board
membership gated on `carryover_stale_age` returning `Some` — measured at only **6 of 142** entries,
hiding the other 136, including every P0 filed the same day (by construction not yet stale).
`rank_carryover` ranks **every** entry passed to it; `stale` is consulted only as one input to
AGING-lane membership (§4). A consumer building a board from this contract must pass the **full**
entry set, not a pre-filtered stale subset, or the fix does not propagate.

`carryover_stale_age` remains the **single** definition of the staleness predicate. This contract
does not reimplement it and a consumer must not either — it is only ever called once, upstream of
`rank_carryover`, to produce each `CarryoverVerdict.stale` flag.

---

## 3. `CarryoverRanking`

The per-entry wire shape `rank_carryover` returns, one per input entry:

```rust
#[derive(Debug, Clone, Serialize)]
pub struct CarryoverRanking {
    pub repo: String,
    pub slug: String,
    pub kind: String,
    pub lane: TriageLane,
    pub priority: Option<u8>,             // omitted from JSON when None
    pub effective_priority: Option<u8>,   // omitted from JSON when None
    pub age_days: Option<i64>,            // omitted from JSON when None
    pub stale: bool,
    pub unmet_blocks: Vec<String>,        // omitted from JSON when empty
    pub clears_when_satisfied: bool,
    pub finding_id: Option<String>,       // omitted from JSON when None
}
```

| Field | Type | Meaning |
|---|---|---|
| `repo` | `String` | Owning repo slug. |
| `slug` | `String` | The carryover's own slug, unique within `repo`. |
| `kind` | `String` | Authored kind (`deferred`, `known_issue`, etc.) — passed through verbatim. |
| `lane` | `TriageLane` | The assigned lane — see §4. |
| `priority` | `Option<u8>` | The entry's own authored priority (0=hottest, 3=coldest), verbatim, never reconciled across repos. `None` when absent. |
| `effective_priority` | `Option<u8>` | Priority after min-propagation across `blocks[]` (§5). `None` when the entry has no own priority and no hotter transitive target. |
| `age_days` | `Option<i64>` | Days since the carryover's staleness anchor. `None` when currently snoozed or the anchor date does not parse. |
| `stale` | `bool` | Whether `carryover_stale_age` (against the kind's threshold) returned `Some` — the sole input to AGING membership. |
| `unmet_blocks` | `Vec<String>` | Every unmet `blocks[]` target key (`"{repo}:{id}"` for a `Block` edge, `"external:{what}"` for an `External` edge). Empty means fully met or no `blocks[]` at all. |
| `clears_when_satisfied` | `bool` | `true` iff the source verdict's `lane == CarryoverLane::Cleared` — read verbatim, **never re-evaluated** here (`MV.ticket.clears-when-evaluation` already did that work). |
| `finding_id` | `Option<String>` | Cross-repo finding identity, verbatim from the source item. |

Fields marked "omitted from JSON when …" use `#[serde(skip_serializing_if = …)]`; a consumer
deserializing this shape must treat the field's absence identically to an explicit `null`.

---

## 4. The four lanes

Assigned by `assign_triage_lane` in this exact priority order — every entry lands in **exactly
one** lane:

| Order | Lane | Membership | Within-lane order |
|---|---|---|---|
| 1 | **BLOCKING** | `unmet_blocks` is non-empty | `effective_priority` ascending (0 hottest; absent sorts last), then `age_days` descending |
| 2 | **HOT** | authored `priority` is `Some(0)` or `Some(1)`, and not already BLOCKING | `priority` ascending (absent sorts last), then `age_days` descending |
| 3 | **AGING** | `stale == true` (i.e. `priority` 2, 3, or absent, and old enough) | `age_days` descending |
| 4 | **STANDING** | everything else — no priority and no `blocks[]` edges at all | `age_days` descending |

Every lane's secondary/only key resolves ties with `(repo, slug)` ascending, so output order is
**fully deterministic** across calls and independent of input order or hash-map iteration.

**STANDING** exists so permanent, always-true constraints (e.g. "`planning/` is a symlink, pass
`-L`") stop competing with actionable work — it is a low-frequency re-affirm lane, not a backlog.

**Absent priority/effective-priority sorts last within a lane**, matching
`effective_priority_for`'s existing `u8::MAX`-absent convention for blocks (`src/brain/emit.rs`).
A consumer must not invent a different absent convention.

Blocking-ness is **never authored** — there is no `blocking: bool` field anywhere in this crate or
its wire shape. It is always derived from `unmet_blocks`, itself derived from the entry's
`blocks: Vec<BlockedBy>`.

---

## 5. Effective priority across `blocks[]`

`carryover_effective_priorities(entries, block_priorities) -> HashMap<String, u8>` (keyed
`"{repo}:{slug}"`) computes, for every carryover, the minimum of:

```
effective(c) = min(own(c), min{ effective(t) : t in c.blocks })
```

- A carryover gating a hotter block or carryover **inherits that hotness**.
- `BlockedBy::Block { repo, id, .. }` resolves against `block_priorities` first (a block target —
  terminal, never recomputed), then against sibling carryovers by `"{repo}:{slug}"`; an empty
  `repo` falls back to the carryover's own `repo`. An unresolvable target in neither map
  contributes nothing.
- `BlockedBy::External { .. }` contributes **no priority** to this propagation — it has no node
  target — but it **still counts** as an unmet `blocks[]` edge for BLOCKING-lane membership (§4).
  These are two different questions; do not conflate them.
- **Cycle-safe**: memoized DFS with an on-stack recursion guard — a key already being computed
  further up the walk short-circuits to its own priority instead of recursing again. A two-node
  cycle or a self-edge terminates deterministically; it never hangs or panics.
- **Absence, not `u8::MAX`**: an entry with no own priority and no hotter transitive target is
  **absent** from the returned map — `.get(key).copied()` naturally reads `None`. This is the same
  absent-not-sentinel convention `state::effective_priorities` uses for blocks.
- This pass **reuses** the existing reverse-topological min-propagation shape used for blocks
  (`state::effective_priorities` / `effective_priority_for` / `block_graph`'s recursion guard); it
  is not a second, independent implementation, and it never changes a block's own effective
  priority.

---

## 6. Rules a consumer MUST honour

1. **Call `rank_carryover`; never re-derive the ranking.** Lane assignment, effective-priority
   propagation, and sort order are all mev's to own. A consumer projects the returned
   `Vec<CarryoverRanking>` as-is.
2. **Never author a `blocking: bool` field.** If a consumer's own DTO needs a boolean, derive it
   from `!unmet_blocks.is_empty()` at the boundary — never persist it.
3. **Per-repo priority divergence is information, never reconciled.** The same `finding_id` may
   carry `priority: 0` in one repo and `priority: 2` in another — that is a legitimate signal about
   differing local impact, not a conflict to average or overwrite. Dedup merges the *claim* (via
   `FindingCluster`), never the *priority*.
4. **Suggestions are never auto-merged.** `CarryoverReport.suggestions` (heuristic
   candidate-duplicate pairs) and `FindingCluster` are read-only signal for a human; nothing the
   *ranking* this contract defines deletes, snoozes, or merges an entry automatically. (The
   separate `mev carryover --dispose` write path — §6a below — acts on the CLEARED lane this
   ranking produces, but it neither ranks nor reconciles anything itself.)
5. **Board membership passes the full entry set**, not a staleness-pre-filtered subset — see §2.
6. **`stale` is read only from `carryover_stale_age`**, never reimplemented downstream.

---

## 6a. `--dispose` — the one write path over this ranking

`mev carryover` (plain sweep and `--audit`) is read-only. `mev carryover --dispose`
(`MV.ticket.carryover-dispose`) is the one exception: it re-runs the sweep this contract's
ranking is built from and **moves** every entry the CLEARED lane lands on out of its owning
repo's `carryover[]` and into that repo's `planning/carryover-archive.jsonl` as a
`CarryoverArchiveRow` (okf-core `OK.4.A`) — never a delete; the entry is kept as data, verbatim,
plus `disposed_at`/`reason: cleared`/`reconstructed: false`/`evidence`. A repo whose sweep failed
to evaluate is skipped, not silently disposed as if it had nothing CLEARED. `--dispose` never
implies `--allow-exec`. Both the `state.json` removal and the archive append are written
together as one atomic step, and the command prints the exact `git commit -o <pathspec>`
covering both files so an operator commits them together. `--dispose --dry-run` runs the
identical code path with both writes suppressed. Full flag reference and examples: the
`carryover` entry in [`docs/cli.md`](cli.md), `--dispose` subsection.

---

## 6b. `W_STATE_CARRYOVER_ALREADY_SATISFIED` — the already-satisfied gate

This is a **validation diagnostic**, not part of the ranking wire shape above — it does not change
`rank_carryover`'s exported surface and does not bump this document's contract version. It is
documented here because it is this repo's other carryover-facing check and this is the repo's
existing carryover doc.

**The rule.** HQ's `CLAUDE.md` carryover-routing section already states it in prose: *"Never author
a typed `clears_when` that is already satisfied — it retires the entry on its first `mev carryover`
sweep while the finding is still live, which is worse than no predicate."* Before this gate, nothing
enforced it. Measured on the brain 2026-08-19: of 5 `CLEARED` entries, 3 were false, and 2 of the 3
were already satisfied the moment they were authored.

**What it checks.** `validate_brain_state` (the `state` pass) evaluates every file's
`carryover[].clears_when` once, up front, into a `CarryoverReport` — with `allow_exec: false`, so a
`command_exits_zero` predicate is **never executed** by the validator. It never fires for
`command_exits_zero` entries; they land `NotEvaluable` under exec-disabled evaluation, which is the
correct and deliberately safe outcome, not a gap to "fix" by turning exec on in a validator that runs
in CI and the nightly routine. `check_carryover_already_satisfied`
(`src/brain/state.rs`) then walks every `carryover[]` entry whose verdict in that report landed in
`CarryoverLane::Cleared` — i.e. the predicate is satisfied *right now*, while the entry is still
present and un-disposed — and emits one `W_STATE_CARRYOVER_ALREADY_SATISFIED` warning per such
entry, naming the entry's `slug`, the rendered predicate, and why it matched (from the verdict's own
matched refs, never re-derived).

**This is distinct from the sweep's healthy `CLEARED` lane.** `mev carryover`'s plain sweep reports
`Cleared` as a normal, actionable-for-disposal outcome. This gate speaks only about an entry that is
*still live* and *already* satisfied — by construction that combination means either (a) the entry
was already resolved when it was filed and should never have been added, or (b) it is predicated on
the wrong observable. Both are author errors, and the diagnostic's own message says so — it is worded
distinctly from a `Cleared` sweep result so the two are never confused in output.

**Severity is Warning, and stays Warning.** `Report::is_failure` counts only `Error`, so
`validate-brain --state` still exits 0 in this diagnostic's presence. Promoting it to error is
explicitly out of scope until the fleet's predicate-less entries have been triaged — this is a
detection mechanism first, not yet an enforcement gate.

**Two sub-classes, one code.** Both were measured on the brain on 2026-08-19 and need different
fixes, so the gate names each as a refinement of the same `W_STATE_CARRYOVER_ALREADY_SATISFIED`
message rather than as separate codes — a grep for the code still finds every instance:

- **Sub-class A — unanchored `file_contains` matching prose.** Fires when the satisfied predicate is
  `file_contains` and its `pattern` has no leading newline (this corpus's convention for anchoring a
  pattern to a specific YAML frontmatter field). Ground truth: `postgres-14-17-cleanup-pending` was
  cleared on 2026-08-19 by the bare substring `status: archived`, which also occurs in the runbook's
  own Phase 7 prose describing the flip, while the frontmatter still read `status: draft`. **Fix:**
  anchor the pattern with a leading `\n`.
- **Sub-class B — a path predicate scoped to another machine.** Fires when the satisfied predicate is
  `file_exists` or `file_contains` and the entry's own `text` mentions another machine (matched
  case-insensitively against `mini`, `mac mini`, `on that machine`, `remote`, `another machine`).
  Ground truth: `client-wild-trail-photo-missing-on-mini` (a tier-scoped entry, `{repo: null, tier:
  "client"}`) was reported `CLEARED` on 2026-08-19 because a repo-relative path resolved on the dev
  checkout while the finding was actually about the Mac Mini, which was still broken. **Fix:**
  re-predicate on something the running host can actually observe — or, as this entry now does,
  fall back to free prose so it lands not-evaluable instead of falsely clearing.

**The remedy is always re-predicate, never delete.** Deleting the entry throws away the finding it
was filed to track; naming the predicate imprecisely is the author error the gate exists to catch,
not a reason to remove the record of the finding itself.

---

## 7. Consumer re-pin instructions

Following the `docs/decisions/D20-shared-data-contract.md` pattern (see
`core/orchestrator/docs/data-contract.md` / `core/bastion/docs/data-contract.md` for the model):

1. A consumer (e.g. `bastion`) pins its own `docs/data-contract.md` (or a dedicated
   `docs/carryover-contract.md`) to a specific version of **this** document.
2. When this document's version bumps, the consumer's `/log-work` checklist prompts a re-pin: read
   this file's changelog, update the consumer's pinned version line, update any field mappings, and
   update the affected Rust types.
3. A version bump on this side **requires**: a changelog entry here (below), a note in mev's
   `planning/status.md`, and — once a consumer exists that has pinned it — a matching changelog
   entry in the consumer's own contract doc plus a note in the consumer's `planning/status.md`.
4. Semver: **patch** = wording/clarification, no shape change. **minor** = additive,
   backward-compatible (a new optional field, a new lane). **major** = breaking (rename/remove a
   field, change a lane's membership rule, change the sort order).

---

## Changelog

| Version | Date | Change |
|---|---|---|
| 1.0.0 | 2026-08-09 | Initial contract: `rank_carryover`, `CarryoverRanking`, `TriageLane`, `CarryoverVerdict`, `FindingCluster` exported surface; the four-lane membership/ordering table; effective-priority min-propagation semantics across `blocks[]` (cycle-safe, absent-not-sentinel); the staleness-is-not-a-membership-gate rule; and the consumer rules (never re-derive, never author `blocking`, never reconcile divergent priorities, never auto-merge suggestions). No consumer has pinned this version yet — `BA.ticket.carryover-triage-dto` is the first planned pin. |
