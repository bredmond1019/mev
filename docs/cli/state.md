---
type: Reference
title: mev CLI — state and derivation commands
description: The commands that read authored state and regenerate every derived surface from it — emit-state, its revision history, the single-block status writer, and the corpus manifest.
doc_id: cli-state
layer: [factory]
project: mev
status: active
keywords: [emit-state, derivation, state.json, revision history, manifest]
related: [cli-reference, cli-epics, architecture]
---

# mev CLI — state and derivation commands

Part of the [CLI reference](../cli.md).

## What this page is for

The Brain has two kinds of state. **Authored** state is what a human or agent wrote by hand — a
block's `status`, a `carryover[]` entry. **Derived** state is everything generated from it: focus
lines, rollup tables, project caches, wave tables, the Operating Board. The derivation runs one
way only, and these are the commands that drive it.

The rule that follows from that: **never hand-edit a derived surface.** Change the authored input,
then re-derive. A hand-edit is overwritten on the next run and looks like data loss.

| Command | Does |
|---|---|
| [`emit-state`](#emit-state---write-path) | Regenerates **every** derived surface from authored state |
| [`state-history`](#state-history-path---restore-seq) | Lists, and can restore, the revisions `emit-state` records |
| [`set-block-status`](#set-block-status-repoid-status-path---write---force-operator-gate---scope-slug) | Flips one block's authored `status`, then re-derives |
| [`create-block`](#create-block---from-file-path---write---scope-slug) | Files a new block/ticket/chore record, then re-derives |
| [`demote-block`](#demote-block-repoid-path---write---scope-slug) | Parks an existing block into `backlog[]`, record intact — `create-block`'s inverse |
| [`promote-block`](#promote-block-repoid-path---write---scope-slug) | Restores a `parked` backlog entry back into `tracks[]` — `demote-block`'s inverse |
| [`manifest`](#manifest---pretty-path) | Emits a JSON manifest of every file in the corpus |

## Quickstart

Run these in a **terminal**. Start with the dry run — every one of these writes fleet-wide.

```bash
# See what would change; writes nothing
mev emit-state

# Regenerate every derived surface across the WHOLE corpus, not just this repo
mev emit-state --write

# Flip one block, which re-derives internally
mev set-block-status mev:MV.1.A closed --write

# File a new block/ticket/chore record from a JSON payload, which also re-derives
mev create-block --from payload.json --write

# Park an existing block into backlog[] — its planning/blocks/<ID>.json record is untouched
mev demote-block mev:MV.10.A --write

# Restore it — the inverse
mev promote-block mev:MV.10.A --write

# If a write went wrong, list revisions and roll back
mev state-history core/mev/planning/state.json
mev state-history core/mev/planning/state.json --restore 3
```

**`--write` rewrites the whole corpus, not your repo.** Every write verb here re-runs the full
derivation internally, so a stale installed `mev` silently rewrites surfaces in an old format.
Install before you write: `cargo install --path .`

**Commit immediately after any `--write`.** Sibling lanes read these files; an uncommitted
regeneration is invisible to them and gets clobbered by the next agent that writes the same file.

## Commands

### `manifest [--pretty] [path]`

Emit a JSON manifest of every file in the Brain corpus.

```bash
mev manifest [--pretty] [path]
```

| Argument / Flag | Default | Description |
|---|---|---|
| `path` | `.` | Path to search from when locating `brain.toml` (walks up to find it) |
| `--pretty` | off | Emit pretty-printed (indented) JSON instead of compact JSON |

Resolves `brain.toml` by walking up from `path`. If no `brain.toml` is found, the process
exits 1 with an error message on stderr.

The output is the manifest JSON written directly to stdout — there is no `--json` envelope
wrapper; the output *is* JSON.

#### Output shape

```json
{
  "version": "1",
  "root": "/path/to/brain",
  "entries": [
    {
      "rel": "planning/status.md",
      "scope": "brain",
      "doc_id": "mev-status",
      "doc_type": "ProjectStatus",
      "title": "MEV Status",
      "description": "Current project state for the mev validator.",
      "layer": ["factory"],
      "project": "mev",
      "status": "active",
      "keywords": ["mev", "status", "validator"]
    },
    {
      "rel": "README.md",
      "scope": "brain",
      "doc_id": null,
      "doc_type": null,
      "title": null,
      "description": null,
      "layer": null,
      "project": null,
      "status": null,
      "keywords": null
    }
  ]
}
```

| Field | Type | Description |
|---|---|---|
| `version` | string | Schema version — currently `"1"` |
| `root` | string | Display path of the HQ root used for the crawl |
| `entries` | array | All corpus files, in walk order |
| `entries[].rel` | string | Path relative to the HQ crawl root (forward-slash separated) |
| `entries[].scope` | string | Stable scope slug of the owning registry unit (e.g. `"brain"`, `"mev"`) |
| `entries[].doc_id` | string \| null | OKF `doc_id` field; `null` when not present in frontmatter |
| `entries[].doc_type` | string \| null | OKF `type` field (serialized as `doc_type`); `null` when absent |
| `entries[].title` | string \| null | OKF `title` field; `null` when absent |
| `entries[].description` | string \| null | OKF `description` field; `null` when absent |
| `entries[].layer` | array \| null | OKF `layer` field (closed-set list); `null` when absent |
| `entries[].project` | string \| null | OKF `project` field; `null` when absent |
| `entries[].status` | string \| null | OKF `status` field; `null` when absent |
| `entries[].keywords` | array \| null | OKF `keywords` field (3–7 free-form terms); `null` when absent |

Files without parseable frontmatter appear in the manifest with all metadata fields set to
`null` (graceful degradation — the OKF validator reports the error separately).

#### Exit codes

| Code | Meaning |
|---|---|
| `0` | Manifest emitted successfully |
| `1` | `brain.toml` not found, or a runtime error prevented crawl completion |

**Examples:**

```bash
# Compact JSON from the current directory
mev manifest

# Compact JSON from an explicit brain root
mev manifest ~/Dev/agentic-portfolio

# Pretty-printed JSON
mev manifest --pretty

# Pretty-printed JSON from an explicit brain root
mev manifest --pretty ~/Dev/agentic-portfolio

# Pipe compact JSON into jq
mev manifest | jq '.entries | length'
```

---

### `emit-state [--write] [path]`

Regenerate all derived views in the Brain corpus from the authored `tracks[]` DAG and write them in place (with `--write`) or report what would change (dry-run, without `--write`).

`mev emit-state` is the **single derivation engine** that `/log-work` shells out to for regenerating leaf `focus` fields, the brain `repos[]` / `cross_repo[]` rollup, brain `focus`, the master-plan wave/dependency tables, the per-project cache docs (focus line + `synced_from` watermark), the tier sub-brain rollup tables, the HQ Operating Board, the HQ unified priority board, and (MV.13.A, record format updated MV.17.A) the cross-repo `planning/lane-segments.json` artifact — every discovered `lane-<name>.json` record's blocks segmented into `{roadmap, lane, segment, position}` runs by each block's *authored* `repo`/`origin_roadmap` fields (ownership is no longer resolved via an `OwnerIndex`, and a block claimed by two lane files renders once per appearance under its own `origin_roadmap` rather than being suppressed). Because the validator's `check_focus_drift` and `check_rollup` share the same `derive_focus` / `derive_rollup` functions, running `mev emit-state --write` followed by `mev validate-brain --state` on the same corpus will report zero `W_STATE_FOCUS_DRIFT` and zero `W_STATE_ROLLUP_DRIFT` — the emit is, by construction, the fixed point of the drift check across every generated surface.

```bash
mev emit-state [--write] [--scope <repo>] [--require-fresh] [path]
```

| Argument / Flag | Default | Description |
|---|---|---|
| `path` | `.` | Path to search from when locating `brain.toml` (walks up to find it) |
| `--write` | off | Write the derived views in place. Without this flag the command is a dry-run |
| `--scope <repo>` | unset (whole corpus) | Limit regeneration to one repo's own derived surfaces plus the rollups it feeds — nothing else. Omit for today's default full-corpus behaviour, byte-for-byte unchanged. |
| `--require-fresh` | off | Promote a `toolchain-freshness` `Drift` verdict from a warning to a hard failure: no write performed, non-zero exit. Convenience alias for setting the `MEV_REQUIRE_FRESH` env var before the write runs — see [Toolchain freshness check](#toolchain-freshness-check-on-write) below. `NotEvaluable` never triggers this, only a genuine `Drift` does. |

#### Toolchain freshness check on `--write`

Every `--write` path chains through the same `emit_state` function (`set_block_status`,
`defer-epic`/`resume-epic`/`complete-epic`, `close-operator-gate`, `approve`/`reject` all call it
internally), so the toolchain-freshness verdict is checked once, at the top of that function,
before any plan is computed — this is the single choke point that covers every writer named above,
not just `emit-state` itself.

The check queries **every registered corpus writer** — `self` (this binary, compiled-in stamp) plus
every entry in `toolchain::CROSS_BINARY_WRITERS` (today: `bastion`, queried via `bastion
--build-stamp`) — and aggregates worst-wins: any writer `Drift` makes the overall verdict `Drift`. A
writer that does not implement `--build-stamp` (not found on `PATH`, non-zero exit, or malformed
JSON) reports `NotEvaluable` and is named as such — **never silently `Pass`**. `NotEvaluable` never
warns or blocks; only a genuine `Drift` does.

- **Default (interactive) path** — a `Drift` verdict prints a loud stderr banner naming the stale
  binary(ies) and their stamped-vs-live SHAs, then the write proceeds exactly as before:

  ```
  ================ TOOLCHAIN DRIFT ================
  mev emit-state --write: at least one registered writer's binary does not match its source tree.
  bastion: stamped a1b2c3d, live head 9f8e7d6
  Rebuild/reinstall the stale binary before trusting this write. Set MEV_REQUIRE_FRESH=1 (or pass
  --require-fresh to `mev emit-state`) to make this a hard failure instead of a warning.
  ===================================================
  ```

- **Unattended path** — set `MEV_REQUIRE_FRESH` to `1` or `true` (matched case-insensitively; any
  other value, including unset, empty, or `0`, is not-set) or pass `--require-fresh` to `mev
  emit-state` (a convenience alias that sets the same env var before the write runs, so
  `emit_state`'s own signature — and its many existing callers — needs no new parameter). A `Drift`
  verdict then returns `E_TOOLCHAIN_STALE` before any plan is computed or applied: no write
  performed, non-zero exit.

  ```bash
  MEV_REQUIRE_FRESH=1 mev emit-state --write
  # or
  mev emit-state --write --require-fresh
  ```

On a fresh binary (no drift), behaviour is byte-identical to before this check existed.

#### `--scope <repo>` — per-repo regeneration

Unscoped, `emit-state --write` regenerates the derived surfaces of **every** registered repo on
every run — a single `/log-work` in one sub-repo dirties dozens of files across the whole corpus.
`--scope <repo>` narrows a `--write` to exactly the surfaces that one repo feeds, derived
mechanically from the `[[repos]]` registry in `brain.toml` (never hardcoded):

- the repo's own leaf `planning/state.json`,
- its `cache_doc` (e.g. `docs/projects/<slug>.md`),
- its tier container's rollup `status.md` (the `[[repos]]` entry whose `slug` matches this repo's
  `tier`), when that tier resolves to a distinct registered entry, and
- the HQ root's `status.md` (the Operating Board) — every repo feeds this one.

Every other repo's files are left byte-identical. A scoped run never blanks or truncates a repo it
did not visit — rollups preserve every row it didn't touch. An unknown `--scope` slug fails fast
with `E_EMIT_UNKNOWN_SCOPE`, naming every valid slug, before any planner runs or any file is
touched.

This also holds for the three corpus-wide derived artifacts — `planning/lane-segments.json`, the
lane frontier, and lane-segment availability — none of whose target paths are among the four
surfaces above: a scoped `--write` now writes none of them rather than always regenerating them in
full (`MV.ticket.emit-state-write-is-corpus-wide-and-unscoped`). Omit `--scope` (or run the
periodic full reconciliation below) to regenerate these.

**Operating guidance:** reach for `--scope <repo>` from a single sub-repo's own workflow commands
(`/log-work`, `/start-block`, `/blocked`, …) where only that repo's state changed — it keeps the
diff local and avoids cross-repo churn when several agents are working concurrently. Leave the
periodic full reconciliation (cron-bound `routine.sh`, or any run meant to catch drift across the
whole corpus) unscoped — only a full run recomputes every cross-repo edge (`cross_repo[]`, the
unified board's cross-repo priority sort, epic relationships) that a single repo's scope cannot see.

```bash
# Regenerate only mev's own derived surfaces (its state.json, cache doc, tier rollup, HQ board)
mev emit-state --scope mev --write

# Unknown slug: fails fast, names the valid slugs, writes nothing
mev emit-state --scope not-a-repo --write
```

#### Advisory lock on `--write`

`--write` (scoped or not) takes an exclusive advisory lockfile at `<root>/.mev-emit.lock` for the
duration of the run, recording the owning pid. This guards against concurrent `emit-state --write`
invocations interleaving writes to the same derived file — a real risk given how many workflow
commands shell out to `emit-state --write`, and one `E_EMIT_LINKED_WORKTREE` does not cover: that
guard only catches a linked git *worktree*, not the symlinked `planning/` vaults (D46) two agents
in different sub-repos can both be writing through at once.

A second concurrent `--write` polls briefly for the lock to free up, then fails with
`E_EMIT_LOCK_HELD` (naming the holder's pid) and writes nothing rather than interleaving. A
lockfile whose owning process is no longer alive is reclaimed automatically instead of blocking
forever. Dry-run (no `--write`) never takes the lock and is unaffected by contention.

#### Quiesce lease on `--write` (`--agent`, `--lock-dir`)

`.mev-emit.lock` only stops two writers from landing at the same instant — it has no way to
express "I am reconciling `state.json` files right now and validating the result; do not
regenerate the corpus out from under me." A sibling lane declares exactly that by holding an
exclusive **lease** (`.claude/workflows/lease.schema.json`, owned by base-template) at
`<lock_dir>/leases/*.json`. Every corpus-wide write verb now consults that lease store
immediately before it would take `.mev-emit.lock`, and refuses instead of writing when a
`scope: fleet` (or matching `scope: repo`) exclusive lease is held by someone else.

**Covered verbs** — every command in this doc that takes the advisory lock takes this check
first: `emit-state --write`, `set-block-status --write`, `defer-epic`/`resume-epic`/
`complete-epic`/`sync-epics --write`, `close-operator-gate`, `approve`, `reject`,
`state-history --restore`, and `normalize-op-slugs --write`.

**`--agent <name>`** — identifies the calling lane/agent to the quiesce check. A lease whose
`agent` field matches this value **never refuses that caller**, even while the lease is held and
non-stale (the self-exemption `fleet_concurrency_check.py`'s `register --agent` already
implements — a lane reconciling state under its own lease must still be able to write). **Omit
`--agent` and any live exclusive lease refuses the call, including one the caller itself holds
under a different identity** — an unidentified caller can never be self-exempted, by design;
silently exempting it would defeat the guard.

**`--lock-dir <path>`** — where the lease store (and `.mev-emit.lock`) live. Resolved in this
order, identical to `check_lane_agents.py::resolve_lock_dir` (do not expect a different answer
from the two tools): explicit `--lock-dir`, else the `FLEET_LOCK_DIR` environment variable, else
`<brain_root>/.fleet-locks`.

**Two degrade rules, both deliberate — read as bugs if you don't know they're rules:**
- A missing or unreadable `leases/` directory resolves to **clear**, never to a hold. An
  unreadable lease store must not wedge every write verb in the fleet.
- A **stale** lease (past its staleness threshold, judged on `heartbeat` when present else
  `acquired_at`) never refuses. A lease that looks live but is actually abandoned is not a quiet
  window.

A refusal prints and exits non-zero **before anything is written** — the quiesce check runs
ahead of `.mev-emit.lock` being taken, so a quiesced write never touches the lock either.

```bash
# Reconciling state.json under your own lease: still writes, because --agent self-exempts it
mev emit-state --write --agent engine-rs-e3

# No --agent: refused by ANY live exclusive lease, even your own
mev emit-state --write

# Point at a scratch lease store instead of the real fleet .fleet-locks
mev set-block-status mev:MV.10.A closed --write --lock-dir /tmp/fixture-locks --agent test-agent
```

#### Derived views updated

- **Leaf `state.json`** (`kind == "project"`): regenerates `focus` — `now` = blocks with `status: in_progress`; `next` = ready open blocks in `wave` order; `blocked` = open blocks with an unmet `depends_on`, each carrying the unmet `blocked_by[]` subset. Authored `tracks[]` and all other fields survive the round-trip unchanged.
- **Brain `state.json`** (`kind == "brain"`): regenerates `repos[]`, `cross_repo[]` (cross-repo `depends_on` edges), and the brain file's own `focus`. Authored `tracks[]`, `backlog[]`, and `tiers[]` are left untouched.
  - `repos[]` is **tier-scoped**: a brain file whose `repo` slug matches a `tier` value in `brain.toml` (e.g. `core`) scopes to only that tier's `[[repos]]`; a brain file whose `repo` matches no tier (the HQ root) scopes to every repo. See `tier_scope_for`.
  - `repos[]` is **non-destructive**: for each in-scope repo, if a loadable child `state.json` exists, its headline is derived as before (`RepoRollup.tier` populated from config); if not, but the brain file already carries a `repos[]` entry for that slug, the entry is **preserved verbatim** (with `tier` backfilled); only when neither exists is a tier-tagged empty stub emitted. A malformed or not-yet-authored child `state.json` can therefore never silently drop a repo out of the rollup.
  - `focus.now/next/blocked` is derived as the **repo-tagged union** of the in-scope children's own derived `focus` (each block carries its source `repo`), in config-repo order then within-child order, deduplicated by `(repo, id)`. Repos with no loadable child contribute nothing to `focus` (they still surface in `repos[]` via the preserve/stub branch).
- **`master-plan.md` wave tables**: splices a rendered wave/dependency Markdown table between the `<!-- BEGIN generated:wave-table -->` and `<!-- END generated:wave-table -->` sentinels. All narrative lines outside the sentinels are preserved verbatim. Re-running the emit is idempotent — if the splice produces no change, no `EmitAction` is recorded.
- **`master-plan.md` body** (`MV.ticket.master-plan-generator`, same file as the wave table but a separate sentinel region): splices an initiative index plus per-phase block sections — each block's title, description, status (`open` when absent), wave (`—` when absent) and dependency edges — between `<!-- BEGIN generated:master-plan-body -->` and `<!-- END generated:master-plan-body -->`. Initiative labels are read from optional `planning/blocks/<ID>.json` records; malformed records warn and are skipped. A repo whose `master-plan.md` is missing, carries no sentinel pair, or has no blocks is skipped (`W_EMIT_NO_SENTINEL`, or silently when there is simply nothing to render) — the sentinel pair is never created for you, so the generator stays inert until a repo opts in by adding it.
- **Project-cache docs** (`docs/projects/<slug>.md`, one per leaf project repo): splices the derived focus headline into the `<!-- BEGIN generated:project-cache -->` / `<!-- END generated:project-cache -->` sentinels and reconciles the doc's OKF frontmatter `synced_from` field to the child `state.json`'s `updated` watermark. A repo with no matching `[[repos]]` entry, or whose entry has a blank `cache_doc`, is silently skipped (nothing to target).
- **Tier rollup tables** (each tier sub-brain's sibling `status.md`): splices a rendered per-repo now/next/blocked rollup table into the `<!-- BEGIN generated:tier-rollup -->` / `<!-- END generated:tier-rollup -->` sentinels. Only brain files scoped to a single tier (`tier_scope_for` resolves to `TierScope::Tier`) are targeted — the HQ root (`TierScope::All`) is skipped by this planner.
- **HQ Operating Board** (the HQ brain's `status.md`): splices a rendered NOW/NEXT/BLOCKED board across every registered repo into the `<!-- BEGIN generated:hq-board -->` / `<!-- END generated:hq-board -->` sentinels.
- **HQ unified priority board** (the same HQ brain's `status.md`, independent sentinel region): splices a priority-ranked NOW/NEXT/BLOCKED/DUE-SOON board into the `<!-- BEGIN generated:unified-board -->` / `<!-- END generated:unified-board -->` sentinels. Rows are tagged `[BIZ]`/`[ENG]` by the source repo's configured tier; `NEXT` is stably re-sorted by `(effective priority asc, due asc)` (absent values last, wave order as the implicit tiebreak). Effective priority (MV.7.A) is computed by `effective_priorities` via reverse-topological `min`-propagation over the `depends_on` DAG, so a block with no own priority that gates a hotter dependent inherits that dependent's priority and floats to the top instead of sorting last; it falls back to the block's own raw `priority` when no hotter dependent exists. `DUE-SOON` lists blocks due within 14 days (overdue included and annotated) sorted by due date ascending.
- **Attention board** (every brain-level `status.md`, tier-scoped): splices the stale-item board into the `<!-- BEGIN generated:attention -->` / `<!-- END generated:attention -->` sentinels. Unlike the boards above (HQ root only), this emits for **both** scopes: the HQ root (`TierScope::All`) unions `carryover[]` from every loaded repo/tier plus the whole HQ `backlog[]`; each tier sub-brain (`TierScope::Tier`) shows its own tier's leaf-repo carryover (plus the tier brain's own) and the HQ backlog nodes whose `repo` belongs to that tier. Seven lanes total: four **carryover triage lanes** — `BLOCKING` · `HOT` · `AGING` · `STANDING` (`MV.ticket.carryover-triage-ranking`) — followed by Aging backlog · Orphaned captures · Stale distilled knowledge, each row `[<repo>]`-tagged. See [Carryover triage lanes](#carryover-triage-lanes) below for how the first four are populated and ordered; the latter three are unchanged — sorted oldest-first, showing only items past their `[attention]` threshold (the visible twin of `W_STATE_BACKLOG_STALE`/`W_DISTILL_STALE`). The fourth lane (distill-freshness-lane) reads each repo's `knowledge.md`/`memory.md` once (cached across boards) and lists D35-distilled entries whose `distill_stale_age` exceeds the `[attention]` `knowledge_days`/`memory_days` threshold, capped at 10 rows per board with an "…and N more" tail — the same predicate `check_distill_staleness` fires `W_DISTILL_STALE` on, so the board never shows an entry the warning didn't also flag.

#### Carryover triage lanes

**Board membership no longer gates on staleness alone.** Before `MV.ticket.carryover-triage-ranking`,
the carryover lane was a single age-sorted "Stale carryover" list gated by `carryover_stale_age`
(the visible twin of `W_STATE_CARRYOVER_STALE`) — measured against the live corpus, only **6 of 142**
`carryover[]` entries were stale, so the board hid the other **136**, including every P0 filed that
day. Every non-snoozed `carryover[]` entry is now ranked via the public `rank_carryover` function
(re-exported from `src/lib.rs`; see [carryover-contract.md](../carryover-contract.md) for the
full, versioned, producer-owned contract) and lands in exactly one of four lanes, assigned in this
order so membership is total and mutually exclusive:

| Lane | Membership | Within-lane order |
|---|---|---|
| `BLOCKING` | at least one unmet `blocks[]` edge | effective priority of what it blocks, ascending (0 hottest); then age descending |
| `HOT` | authored `priority` 0 or 1, not already `BLOCKING` | priority ascending, then age descending |
| `AGING` | stale (per `carryover_stale_age`), and `priority` 2/3 or absent | age descending |
| `STANDING` | no `priority` and no `blocks[]` | age descending |

`carryover_stale_age` remains the single source of the `stale` flag and feeds only `AGING`
membership plus every row's displayed age — it is never reimplemented for this pass.
`effective_priority` propagates across a carryover's `blocks[]` edges by the same cycle-safe
reverse-topological min-propagation the block dependency graph already uses, so a low-priority
carryover gating a hot block inherits that hotness; ties and cycles resolve deterministically and
never hang.

`STANDING` is a **low-frequency re-affirm lane**, not a backlog: it exists so permanent
constraints (e.g. "`planning/` is a symlink, pass `-L`") stop competing for attention with
actionable P0/P1 work and blocking edges.

Each triage lane is capped at `CARRYOVER_LANE_CAP` (20) rows, oldest/lowest-ranked dropped first
within the lane's own order, with an explicit `- …and N more` line stating the true hidden count
when the lane is over cap — never a silent truncation, matching the existing convention on the
distilled-knowledge lane's cap. `mev carryover --json` remains the uncapped, full-fidelity export
for a consumer that needs every entry.
- **Portfolio `state.json`** (`kind == "portfolio"`): not regenerated at all (no `focus` to derive — these are terminal repos), and skipped entirely by the wave-table splice pass — no `master-plan.md` is expected, so no `W_EMIT_NO_SENTINEL` is raised for these repos.

All of the project-cache, tier-rollup, HQ-board, and unified-board planners share the same fixed-point and sentinel-safety guarantees as the wave-table splice: a target document missing its sentinel pair produces a `W_EMIT_NO_SENTINEL` warning and is left untouched, and re-running the emit over already-emitted content produces no further `EmitAction`/`I_EMIT_WROTE`.

#### Sentinel contract

Every planner splices into its own named marker, using the same `<!-- BEGIN generated:<marker> --> ... <!-- END generated:<marker> -->` pair format. The wave-table example:

```markdown
<!-- BEGIN generated:wave-table -->
| Wave | Block | Title | Status | Depends on |
| --- | --- | --- | --- | --- |
... (generated rows) ...
<!-- END generated:wave-table -->
```

The other planners use their own markers in the same document types: `project-cache` (leaf `docs/projects/<slug>.md`), `tier-rollup` (tier sub-brain `status.md`), `hq-board` (HQ `status.md`), `unified-board` (the same HQ `status.md`, an independent sentinel region alongside `hq-board`), and `attention` (every brain-level `status.md` — HQ and each tier sub-brain — an independent sentinel region).

- Both `BEGIN` and `END` sentinels must be present and balanced; a missing or unbalanced pair causes a `W_EMIT_NO_SENTINEL` warning and the file is skipped — sentinels are never invented into arbitrary prose.
- Re-splicing an already-emitted table is idempotent.

#### Diagnostic codes

| Locator | Severity | Condition |
|---|---|---|
| `W_EMIT_DRY_RUN` | Warning | Planned action (dry-run only; no file written) |
| `I_EMIT_WROTE` | Warning | File written (`--write` mode) |
| `W_EMIT_NO_SENTINEL` | Warning | A target document is missing its marker's sentinel pair (`wave-table`, `project-cache`, `tier-rollup`, `hq-board`, `unified-board`, or `attention`); file skipped |
| `E_EMIT_WRITE_FAILED` | Error | IO error writing a file; causes exit 1 |
| `E_CONFIG_NOT_FOUND` | Error | `brain.toml` could not be located by walking up from `path`; causes exit 1 |
| `E_EMIT_LINKED_WORKTREE` | Error | `--write` invoked from inside a linked git worktree; causes exit 1 |
| `E_EMIT_INCOMPLETE_CORPUS` | Error | `--write` refused because one or more discovered `state.json` files failed to load; causes exit 1 |
| `E_EMIT_UNKNOWN_SCOPE` | Error | `--scope` names a slug with no matching `[[repos]]` entry in `brain.toml`; the message names every valid slug; causes exit 1 |
| `E_EMIT_LOCK_HELD` | Error | `--write` could not acquire the advisory lock at `<root>/.mev-emit.lock` within the timeout because another live process already holds it (names the holder pid); causes exit 1. A lockfile whose owning process is no longer alive is reclaimed automatically instead of blocking forever. Dry-run never takes the lock. **Retry shortly** — this is ordinary contention. |
| `E_QUIESCE_LEASE_HELD` | Error | `--write` refused because a sibling lane's exclusive lease (`.claude/workflows/lease.schema.json`) declares a quiet window over this write (names the holding lane, agent, scope and lease path); causes exit 1, nothing written. **Do NOT retry** — wait for the lease to be released, or pass `--agent <name>` to self-exempt a lease this same caller holds. Distinct condition from `E_EMIT_LOCK_HELD`: that is momentary contention, this is a declared quiet window. See [Quiesce lease on `--write`](#quiesce-lease-on---write---agent---lock-dir). |
| `E_TOOLCHAIN_STALE` | Error | `--write` refused because `toolchain-freshness` reported `Drift` and `MEV_REQUIRE_FRESH`/`--require-fresh` is set; causes exit 1, no write performed. Checked before any plan is computed or applied. See [Toolchain freshness check on `--write`](#toolchain-freshness-check-on-write). |

`--write` refuses to run when `path` resolves to a linked git worktree (e.g. `trees/<slug>/` under a
repo that already has its own main working tree) — `emit-state` resolves every repo's derived-file
paths from `brain.toml`, not from CWD, so writing from a worktree would silently regenerate the
**main checkout's** files instead of the worktree's own copy. The command prints an error naming
the worktree path and exits non-zero (`E_EMIT_LINKED_WORKTREE`) without writing anything. Dry-run
(no `--write`) is read-only and is never gated — it still succeeds from inside a worktree. Run
`--write` from the repo's main working tree instead.

`--write` also refuses to run when the corpus is incomplete: if any discovered `state.json` fails
to load (an `E_STATE_MALFORMED_JSON` diagnostic), every derived view is a cross-repo union
(`repos[]`/`cross_repo[]`, tier rollups, HQ/unified/epic boards, master-plan and epic sequence
tables) — regenerating them from a partial corpus would silently erase the missing repo(s) from
every surface, and rewriting `cross_repo[]` would delete the dangling references that are the only
evidence of the failure. The command pushes `E_EMIT_INCOMPLETE_CORPUS` alongside the underlying
`E_STATE_MALFORMED_JSON` cause, writes nothing, and exits non-zero. Dry-run is unaffected — it is
the diagnostic tool for exactly this situation, and still runs every planner and reports the
`W_EMIT_DRY_RUN` actions that would have been taken. Fix the load failure named by
`E_STATE_MALFORMED_JSON`, then re-run `--write`.

**Examples:**

```bash
# Dry-run from the current directory (reports planned changes, writes nothing)
mev emit-state

# Dry-run from an explicit brain root
mev emit-state ~/Dev/agentic-portfolio

# Write derived views in place
mev emit-state --write

# Write derived views from an explicit brain root
mev emit-state --write ~/Dev/agentic-portfolio

# Write only mev's own derived surfaces (leaf state.json, cache doc, tier rollup, HQ board)
mev emit-state --scope mev --write

# Machine-readable dry-run output
mev --json emit-state

# Machine-readable write output
mev --json emit-state --write ~/Dev/agentic-portfolio
```

#### Revision history

Every `--write` overwrite of an existing file goes through `apply_plan()`'s append-only writer
(see `mev state-history` below): before the new content lands, the file's **prior** content is
snapshotted to `<dir>/.mev-history/<filename>/<seq>__<timestamp>`, then the write itself lands
atomically (temp file in the destination's own directory, then `fs::rename`). Creating a
brand-new file records no revision — there is no prior content to lose. A snapshot/prune failure
emits `W_HISTORY_FAILED` and does not block the primary write; history is a safety net, never a
new way for `emit-state` to fail. Snapshotting is controlled by the `[history]` table in
`brain.toml` (`enabled`, `keep`) — see `docs/brain-toml.md`. Dry-run remains fully side-effect-free:
no history directory is created and nothing is written.

---

### `state-history <path> [--restore <seq>]`

List (or restore) the append-only revision history `apply_plan()` records for one file every time
it overwrites existing content. This is the read-back half of the "Revision history" note on
`emit-state` above — a snapshot nobody can retrieve is inert, so `state-history` is what makes a
bad derived write recoverable.

```bash
mev state-history <path> [--restore <seq>]
```

| Argument / Flag | Default | Description |
|---|---|---|
| `path` | required | The file whose revision history to list or restore (e.g. `planning/state.json`), not a brain root to search from — every other subcommand's `path` walks up looking for `brain.toml`; this one already knows exactly which file's history it wants. |
| `--restore <seq>` | unset (list mode) | Restore revision `<seq>`'s content back to `path` instead of listing. |

#### List mode (default)

Read-only; never takes the advisory lock. Prints that file's revisions **newest first** — seq,
UTC timestamp, byte size:

```
     2  20260804T120501Z  842 bytes
     1  20260804T091203Z  798 bytes
```

A file with no recorded revisions prints an explicit `no revisions recorded for <path>` message
and exits successfully — an empty history is a normal state, not an error. `--json` emits a
compact/pretty JSON array of `{seq, recorded_at, bytes}` records, newest first.

#### `--restore <seq>`

Reads revision `<seq>`, first records the file's **current** on-disk content as a new revision
(so a wrong restore is itself undoable via a second restore), then writes revision `<seq>`'s
content back to `<path>` atomically via the same temp-file + rename helper `apply_plan()` uses.
Prints what was restored and what the pre-restore content was saved as (or the JSON equivalent
under `--json`: `restored_seq`, `path`, `pre_restore_revision`).

Because it mutates the file, `--restore` takes the same advisory lock at `<root>/.mev-emit.lock`
that `emit-state --write` takes (resolved from `path`'s own parent directory, walking up to find
`brain.toml`), and the same linked-worktree guard — refusing to run from inside a linked git
worktree with the same shape of message `emit-state --write` gives. List mode is read-only and
skips both checks, exactly like emit-state's dry-run.

An unknown `--restore <seq>` fails, naming the valid seq range. A path with no revisions and
`--restore` given still exits successfully with the "no revisions recorded" message — there is
nothing to restore.

#### Diagnostic codes

| Locator | Severity | Condition |
|---|---|---|
| `W_HISTORY_FAILED` | Warning | The pre-restore snapshot could not be recorded; the restore itself still proceeds (history is a safety net, never a new way for restore to fail) |
| `E_EMIT_LINKED_WORKTREE` | Error | `--restore` invoked from inside a linked git worktree; refused before the lock is taken |
| `E_EMIT_LOCK_HELD` | Error | `--restore` could not acquire the advisory lock because another live write process already holds it (names the holder pid); a stale lock (owning process no longer alive) is reclaimed automatically instead |
| `E_QUIESCE_LEASE_HELD` | Error | `--restore` refused because a sibling lane's exclusive lease declares a quiet window (names lane/agent/scope/path); checked before the advisory lock is taken. Do not retry — wait for release or pass `--agent` to self-exempt. See [Quiesce lease on `--write`](#quiesce-lease-on---write---agent---lock-dir). |

#### Exit codes

| Code | Meaning |
|---|---|
| `0` | Revisions listed, "no revisions recorded", or restore applied |
| `1` | No revision `<seq>` on disk (names the valid seq range), `E_EMIT_LOCK_HELD`, `E_QUIESCE_LEASE_HELD`, a linked-worktree refusal, or an IO failure reading/writing the file |

**Examples:**

```bash
# List a file's revision history, newest first
mev state-history planning/state.json

# Machine-readable listing
mev --json state-history planning/state.json

# Restore revision 1 (also snapshots the current content first)
mev state-history planning/state.json --restore 1
```

---

### `set-block-status <repo:id> <status> [path] [--write] [--force-operator-gate] [--scope <slug>]`

Set **one** block's authored `status` in its repo's `planning/state.json`. The
block-level counterpart to the epic commands above: those move a whole initiative,
this moves exactly one block and nothing else.

**Status only.** Not `priority`, not `due`, not a generic `set-block-field`. The
narrow surface keeps the caller's contract precise; a generic setter would push
per-field validation to runtime.

**The key is always `repo:id`** — e.g. `mev:MV.10.A` — the same
`"{repo_slug}:{block_id}"` form `global_status_map` and `effective_priorities` use.
Block ids are only unique *within* a repo, so an unqualified id is **rejected**
rather than guessed at.

| Status | Meaning |
|---|---|
| `open` | not started, a candidate for `next` |
| `in_progress` | actively being worked (derives into `focus.now`) |
| `deferred` | parked on the back burner (derives into `focus.deferred`) |
| `closed` | done |
| `wontfix` | terminal, but distinct from `closed` — satisfies a `{type:block}` dependency exactly like `closed`, and is tallied in its own `EpicProgress.wontfix` count so it never inflates the `closed` count in the epic progress line |

> **`blocked` is not authorable, and this command rejects it.** `blocked` is a
> *derived* lane: `emit-state` computes it from a block's unmet `depends_on` edges
> and stamps it onto `focus.blocked[]` entries. Writing it onto a `tracks[]` block
> is exactly what `validate-brain`'s `E_STATE_AUTHORED_BLOCKED` exists to catch, so
> input is validated against `VALID_TRACK_BLOCK_STATUSES` (the four above) and not
> against the wider `VALID_STATUSES`. Passing `blocked` fails with
> `E_BLOCK_BAD_STATUS` and writes nothing.

**Setting a block to the status it already has is a no-op success** — zero actions,
zero diagnostics, exit `0`, nothing written. Running the same `--write` twice leaves
the corpus byte-identical.

**Dry-run by default**, exactly like the epic commands: without `--write` the
proposed edit prints as `W_EMIT_DRY_RUN` and not a byte is touched. A successful
`--write` takes the same advisory lock `emit-state --write` takes, refuses to run
against an incomplete corpus (`E_EMIT_INCOMPLETE_CORPUS`), and then runs
`emit-state --write` so `focus`, the boards and the rollups agree with the new
authored value in the same invocation.

**Who calls this.** The intended caller is an **engine-rs workflow node** invoking
the CLI on bastion-web's behalf — "mark this done", "park this" from the web UI.
`bastion serve` is **read-only by decision (D25)** and stays that way, so the write
lands here in mev, the deterministic writer for the brain corpus. The workflow node
itself is engine-rs work and is not part of this command's contract.

**Starting a block that is operator-gated is refused.** Moving a block to
`in_progress` with `--write` while it carries an unmet `Operator` `depends_on`
edge fails with `E_BLOCK_OPERATOR_GATED` unless `--force-operator-gate` is also
passed. The override itself is refused with `E_FORCE_OPERATOR_GATE_NOT_TTY`
whenever stdin is not a TTY — there is no other bypass, and no priority
threshold exempts a block from the gate. The gate only guards *starting*; moving
an operator-gated block to any other status needs no override.

**`--scope <slug>` narrows only the chained `emit-state` regeneration, not the write itself** —
the authored status flip is always exactly one block in its own repo's `state.json`, scope or not.
Resolved identically to [`emit-state`'s `--scope`](#emit-state---write-path) via
`BrainConfig::scope_dependencies` — same `E_EMIT_UNKNOWN_SCOPE` error and message on an unknown or
blank slug, so the two commands can never drift on what a slug resolves to. Omitting `--scope`
regenerates every derived surface fleet-wide, byte-identical to this command's behavior before
`--scope` existed. Reach for it the same way `emit-state --scope` recommends: when closing a block
in one repo's own workflow and only that repo's derived surfaces need to agree with the new status.

```bash
# What would closing MV.10.A change? (dry run — writes nothing)
mev set-block-status mev:MV.10.A closed ~/Dev/agentic-portfolio

# Apply it, and regenerate every derived view
mev set-block-status mev:MV.10.A closed ~/Dev/agentic-portfolio --write

# Park a single block without touching its epic
mev set-block-status bella:BE.2.C deferred --write

# Mark a block as intentionally not being done (terminal, distinct from closed)
mev set-block-status mev:MV.10.B wontfix ~/Dev/agentic-portfolio --write

# Start a block despite an unmet operator gate (interactive shells only)
mev set-block-status mev:MV.10.C in_progress ~/Dev/agentic-portfolio --write --force-operator-gate

# Machine-readable
mev --json set-block-status mev:MV.10.A in_progress --write

# Close a block, regenerating only that repo's own derived surfaces
mev set-block-status mev:MV.10.A closed ~/Dev/agentic-portfolio --write --scope mev
```

Exit codes: `0` planned (dry-run), applied, or already at the target status · `1`
any error-severity diagnostic or a write failure.

| Diagnostic | Cause |
|---|---|
| `E_BLOCK_BAD_KEY` | the key is not `repo:id` (a bare block id, or an empty half) |
| `E_BLOCK_BAD_STATUS` | the status is not one of the five authorable values — this is what rejects `blocked` |
| `E_BLOCK_NOT_FOUND` | no loaded `state.json` owns that `repo:id`; the message lists the known repo slugs when the repo half is the problem |
| `E_EMIT_INCOMPLETE_CORPUS` | `--write` attempted while at least one `state.json` failed to load |
| `E_EMIT_LOCK_HELD` | another mev write holds the brain-root advisory lock |
| `E_QUIESCE_LEASE_HELD` | a sibling lane's exclusive lease declares a quiet window; do not retry — see [Quiesce lease on `--write`](#quiesce-lease-on---write---agent---lock-dir) |
| `E_BLOCK_OPERATOR_GATED` | `--write`ing a block to `in_progress` while it carries an unmet `Operator` `depends_on` edge, without `--force-operator-gate` |
| `E_FORCE_OPERATOR_GATE_NOT_TTY` | `--force-operator-gate` was passed but stdin is not a TTY |
| `E_EMIT_UNKNOWN_SCOPE` | `--scope` names a slug with no matching `[[repos]]` entry in `brain.toml`; the message names every valid slug |

---

### `create-block --from <file> [path] [--write] [--scope <slug>]`

File a **new** block, ticket, or chore: write `planning/blocks/<BlockID>.json` plus its matching
`tracks[].blocks[]` registration in the target repo's `state.json`. The creation counterpart to
`set-block-status` above — that command moves one *existing* block's status, this one adds a block
that does not exist yet — on the same dry-run/`--write`/`--scope` driver contract.

**The authored fields arrive as a JSON payload via `--from <FILE>`, not per-field flags.**
`block.schema.json` has 15 required fields and several are long prose or arrays
(`what`, `why`, `files`, `acceptance_criteria`), which is unusable as shell arguments. See
[`CreateBlockPayload`](../../src/brain/block_create.rs) for the full payload shape; the vocabularies
it enforces:

| Field | Legal values |
|---|---|
| `kind` | `block` \| `ticket` \| `chore` |
| `sdlc_workflow` | `none` \| `patch` \| `task` \| `run` \| `flow` |
| `model` | `sonnet` \| `gemini-pro` \| `gemini-flash` \| `either` — deliberately **not** the SDLC engines' `{haiku, sonnet, opus}` stage-model set; `"opus"` is not a legal block-record `model` |

**`epics` is payload-only, not a `block.schema.json` field.** The block record itself never carries
it (the schema is `additionalProperties: false`); it drives only the `state.json`
`tracks[].blocks[].epics` registration, which the epic-sequence table renders from. A payload with
no `epics` is refused, never written with an empty list — a block created with no epic renders on
no epic-sequence table.

**`spec_dir` is always derived**, never read from the payload — always exactly
`planning/<BlockID>/`, so a typo in the payload can never diverge from the schema's own pattern
constraint.

**Dry-run by default**, exactly like `set-block-status`: without `--write` the proposed record and
`state.json` edit print and not a byte is touched. A successful `--write` takes the same advisory
lock, and then runs `emit-state --write` so the boards, wave table, and epic-sequence table show the
new block in the same invocation.

**An existing block id is a no-op refusal, never an overwrite.** `E_BLOCK_CREATE_EXISTS` fires and
nothing is written — creation only files new blocks.

**A `depends_on` edge naming a block that does not resolve in the loaded corpus is refused**, with
every unresolved `(repo, id)` named in the error (not just the first) — create the dependency before
the dependent.

**Wave allocation**: `10 * phase` for `kind: block`; for `kind: ticket`/`chore`, the next multiple of
ten past the target track's current max wave (never `max + 1`). The target track is matched by title
— `"Phase {phase}"` for a block, `"Tickets"`/`"Chores"` (exact) for a ticket/chore — and created if
none matches.

**`--scope <slug>` narrows only the chained `emit-state` regeneration**, identically to
`set-block-status --scope` — the new record and its `state.json` registration are always written to
exactly the target repo regardless of scope. Note that `--scope` excludes HQ-level docs outside the
scoped repo's own path (e.g. a cross-repo `master-plan.md` epic-sequence table); omit `--scope` if a
non-repo-local surface needs to see the new block too.

```bash
# See what create-block would write (dry run — writes nothing)
mev create-block --from payload.json ~/Dev/agentic-portfolio

# File it, and regenerate every derived view
mev create-block --from payload.json ~/Dev/agentic-portfolio --write

# File it, regenerating only the target repo's own derived surfaces
mev create-block --from payload.json ~/Dev/agentic-portfolio --write --scope mev
```

Exit codes: `0` planned (dry-run) or applied · `1` unreadable/unparseable `--from` file, any
`E_BLOCK_CREATE_*` payload or plan diagnostic, a write failure, `E_EMIT_UNKNOWN_SCOPE`,
`E_EMIT_LOCK_HELD`, `E_QUIESCE_LEASE_HELD`, or a linked-worktree refusal.

| Diagnostic | Cause |
|---|---|
| `E_BLOCK_CREATE_KIND_ENUM` | `kind` is outside `block`/`ticket`/`chore` |
| `E_BLOCK_CREATE_SDLC_WORKFLOW_ENUM` | `sdlc_workflow` is outside `none`/`patch`/`task`/`run`/`flow` |
| `E_BLOCK_CREATE_MODEL_ENUM` | `model` is outside `sonnet`/`gemini-pro`/`gemini-flash`/`either` — this is what rejects `"opus"` |
| `E_BLOCK_CREATE_MISSING_EPICS` | `epics` is empty or absent |
| `E_BLOCK_CREATE_EMPTY_FIELD` | a required prose field is empty |
| `E_BLOCK_CREATE_BLOCK_NEEDS_PHASE` | `kind: block` with no `phase` |
| `E_BLOCK_CREATE_TICKET_NEEDS_TESTING_STRATEGY` | `kind: ticket` with no `testing_strategy` |
| `E_BLOCK_CREATE_EMPTY_OUT_OF_SCOPE` | `out_of_scope` is empty |
| `E_BLOCK_CREATE_EMPTY_ACCEPTANCE_CRITERIA` | `acceptance_criteria` is empty |
| `E_BLOCK_CREATE_UNKNOWN_REPO` | `payload.repo` matches no loaded `state.json`; the message lists the known repo slugs |
| `E_BLOCK_CREATE_EXISTS` | `payload.id` already exists in the target repo — refused, never overwritten |
| `E_BLOCK_CREATE_DANGLING_DEPENDENCY` | a `depends_on` block-type edge names a `(repo, id)` that does not resolve in the loaded corpus |
| `E_EMIT_LOCK_HELD` | another mev write holds the brain-root advisory lock |
| `E_QUIESCE_LEASE_HELD` | a sibling lane's exclusive lease declares a quiet window; do not retry — see [Quiesce lease on `--write`](#quiesce-lease-on---write---agent---lock-dir) |
| `E_EMIT_UNKNOWN_SCOPE` | `--scope` names a slug with no matching `[[repos]]` entry in `brain.toml`; the message names every valid slug |

---


### `demote-block <repo:id> [path] [--write] [--scope <slug>]`

Park an **existing** block into `backlog[]` while its `planning/blocks/<ID>.json` record stays
exactly where it is — `create-block`'s inverse (`create-block` files a block that does not exist
yet; this removes one that does, without discarding its substance). Same dry-run/`--write`/`--scope`
driver contract as `create-block`/`set-block-status` above, plus `--agent`/`--lock-dir` for the
quiesce lease.

**`planning/blocks/<id>.json` is never written.** `demote-block` only checks the record exists on
disk (to read its `kind` and to have something for the backlog pointer to name) — no action this
plans ever touches the file. The record staying put on disk is the whole feature: a verb that moved
or deleted it would reintroduce the exact loss `backlog[]` used to cause when an operator hand-parked
a block by orphaning its record.

**What it writes**, on `--write`: the target's `tracks[].blocks[]` row is removed, and a `backlog[]`
entry is appended with `status: "parked"`, a `record` pointer at the retained
`planning/blocks/<id>.json`, and enough of the removed row (`parked_block`, `parked_track`) that
`promote-block` can restore it losslessly. `created` is set to today, so a parked entry ages on the
Attention board's backlog lane exactly like any other row — parking a block cannot silently become
permanent. See [`backlog[]`'s schema](../../../../docs/state/state-schema.md#backlog--authored) for the
full field table and D12's design rationale
(`planning/decisions/D12-demote-block-backlog-record-pointer.md`).

**Not HQ-only.** Unlike an `idea`/`ready` backlog node, a `parked` one is written into whichever
repo's `state.json` owns the block being demoted — `mev:MV.10.A` parks into `mev`'s own `backlog[]`,
not HQ's — because the record it points at lives in that repo's `planning/blocks/`.

Reuses `set-block-status`'s key-resolution refusals rather than inventing new ones:

```bash
# See what demote-block would write (dry run — writes nothing)
mev demote-block mev:MV.10.A ~/Dev/agentic-portfolio

# Park it, and regenerate every derived view
mev demote-block mev:MV.10.A ~/Dev/agentic-portfolio --write

# Park it, regenerating only the target repo's own derived surfaces
mev demote-block mev:MV.10.A ~/Dev/agentic-portfolio --write --scope mev
```

Exit codes: `0` planned (dry-run) or applied · `1` `E_BLOCK_BAD_KEY`, `E_BLOCK_NOT_FOUND`,
`E_DEMOTE_BLOCK_RECORD_MISSING`, a write failure, `E_EMIT_UNKNOWN_SCOPE`, `E_EMIT_LOCK_HELD`,
`E_QUIESCE_LEASE_HELD`, or a linked-worktree refusal.

| Diagnostic | Cause |
|---|---|
| `E_BLOCK_BAD_KEY` | `key` is not `repo:id` form (block ids are only unique within a repo, so an unqualified id is never guessed) |
| `E_BLOCK_NOT_FOUND` | no loaded file's `tracks[]` owns that block |
| `E_DEMOTE_BLOCK_RECORD_MISSING` | the block resolves but `planning/blocks/<id>.json` is not on disk — there would be nothing for the backlog pointer to name |
| `E_EMIT_LOCK_HELD` | another mev write holds the brain-root advisory lock |
| `E_QUIESCE_LEASE_HELD` | a sibling lane's exclusive lease declares a quiet window; do not retry — see [Quiesce lease on `--write`](#quiesce-lease-on---write---agent---lock-dir) |
| `E_EMIT_UNKNOWN_SCOPE` | `--scope` names a slug with no matching `[[repos]]` entry in `brain.toml`; the message names every valid slug |

Every diagnostic returns a plan with zero actions — nothing is ever partially written.

---

### `promote-block <repo:id> [path] [--write] [--scope <slug>]`

Restore a `parked` `backlog[]` entry back into `tracks[].blocks[]` — `demote-block`'s inverse. Same
driver contract as `demote-block`. Re-inserts the exact row `demote-block` removed, read back from
the backlog entry's own `parked_block`/`parked_track` snapshot, so no field is lost in the round
trip. The backlog entry is never deleted, matching how an ordinary idea promotion leaves its origin
behind — it flips to `status: "promoted"` with `block` set to the restored id, and its
`record`/`parked_block`/`parked_track` extras are cleared.

```bash
# See what promote-block would restore (dry run — writes nothing)
mev promote-block mev:MV.10.A ~/Dev/agentic-portfolio

# Restore it, and regenerate every derived view
mev promote-block mev:MV.10.A ~/Dev/agentic-portfolio --write
```

Exit codes: `0` planned (dry-run) or applied · `1` `E_BLOCK_BAD_KEY`, `E_BLOCK_NOT_FOUND`,
`E_PROMOTE_BLOCK_NOT_PARKED`, `E_PROMOTE_BLOCK_EXISTS`, `E_PROMOTE_BLOCK_MISSING_SNAPSHOT`, a write
failure, `E_EMIT_UNKNOWN_SCOPE`, `E_EMIT_LOCK_HELD`, `E_QUIESCE_LEASE_HELD`, or a linked-worktree
refusal.

| Diagnostic | Cause |
|---|---|
| `E_BLOCK_BAD_KEY` | `key` is not `repo:id` form |
| `E_BLOCK_NOT_FOUND` | no loaded file's `backlog[]` carries that slug in that repo |
| `E_PROMOTE_BLOCK_NOT_PARKED` | the slug exists but its `status` is not `"parked"` — nothing to restore, or it was already restored |
| `E_PROMOTE_BLOCK_EXISTS` | a block with this id is already registered in the target repo's `tracks[]`; refused rather than overwritten |
| `E_PROMOTE_BLOCK_MISSING_SNAPSHOT` | the entry is `parked` but carries no restorable `parked_block` snapshot, or the snapshot doesn't deserialize as a track block — hand-edited or corrupt state |
| `E_EMIT_LOCK_HELD` | another mev write holds the brain-root advisory lock |
| `E_QUIESCE_LEASE_HELD` | a sibling lane's exclusive lease declares a quiet window; do not retry |
| `E_EMIT_UNKNOWN_SCOPE` | `--scope` names a slug with no matching `[[repos]]` entry in `brain.toml`; the message names every valid slug |

Every diagnostic returns a plan with zero actions — nothing is ever partially written.

---
