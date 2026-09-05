---
type: Reference
title: mev CLI — carryover, backlog and attention commands
description: The commands that sweep the fleet's open findings and queued ideas, detect new findings mechanically, and deliver the ones needing a human.
doc_id: cli-carryover
layer: [factory]
project: mev
status: active
keywords: [carryover, backlog, attention, findings, triage, disposal]
related: [cli-reference, carryover-contract, brain-toml-config]
---

# mev CLI — carryover and attention commands

Part of the [CLI reference](../cli.md). The data contract these read is
[the carryover triage ranking contract](../carryover-contract.md).

## What this page is for

A **carryover entry** is a finding that outlived the session that found it — an unticketed defect,
a deferred follow-on, a drifted document, a transient environment caveat. It lives in a repo's
`planning/state.json` under `carryover[]`, and it carries a `clears_when` predicate saying what
would make it stop being true.

A **backlog entry** is the other half of the same problem: an idea that is queued rather than found.
It lives in the same `planning/state.json`, under `backlog[]`, and since
`okf-core:OK.ticket.backlog-lifecycle-predicates` it carries the same two kinds of predicate —
`clears_when` (the idea is dead) and `ready_when` (it is worth doing now).

These commands answer four questions: **what is still open** (`carryover`), **what is queued and
what became of it** (`backlog`), **what should be open but nobody filed** (`graph-findings`), and
**what needs a human right now** (`attention-queue`).

| Command | Answers |
|---|---|
| [`carryover`](#carryover---repo-slug---include-cross-repo---grep-pattern---json---allow-exec---exec-timeout-secs---audit---window-days---dispose---backfill---dry-run---would-block---trajectory---weeks-days-path) | What is open across the fleet, and which entries have resolved? |
| [`graph-findings`](#graph-findings---json---write-path) | What findings can be detected mechanically rather than noticed? |
| [`attention-queue`](#attention-queue---out-path---notify-only-path) | Which stale items need the operator? |

## Quickstart

Run these in a **terminal**. The plain sweep writes nothing.

```bash
# What is open, fleet-wide
mev carryover

# Just my repo -- and then what that filter HID from me
mev carryover --repo mev
mev carryover --repo mev --include-cross-repo

# Search slug + text, case-insensitively
mev carryover --grep lockfile

# Entries whose predicate has resolved -> archive them (a MOVE, never a delete)
mev carryover --dispose --dry-run
mev carryover --dispose

# The same sweep over queued IDEAS rather than findings. Read-only; it has no --dispose.
mev backlog
mev backlog --repo mev
mev backlog --lane ready          # only the rows worth promoting
```

**A `--repo`-filtered sweep is never the whole picture.** Entries scoped `cross_repo: true` or to a
tier belong to no single repo, so they match no `--repo` filter. A filtered `--grep` returning zero
means *no match in this repo's slice*, not *no such entry* — re-run unfiltered before believing it.

**`cleared` is a recommendation, not an action.** The sweep never deletes anything. Only
`--dispose` moves entries, and it moves them into `planning/carryover-archive.jsonl` rather than
deleting them.

## Commands

### `carryover [--repo <slug>] [--include-cross-repo] [--grep <pattern>] [--json] [--allow-exec] [--exec-timeout <secs>] [--audit] [--window <days>] [--dispose] [--backfill] [--dry-run] [--would-block] [--trajectory] [--weeks <days>] [path]`

Fleet-wide sweep of every discovered `planning/state.json`'s `carryover[]` array. By default
this is **read-only**: it evaluates each entry's `clears_when` predicate where it is
machine-checkable and sorts the fleet into three lanes. `--grep <PATTERN>` narrows the swept
entries to those whose `slug` or `text` matches a case-insensitive regex, so retrieving one
known entry no longer means dumping the whole fleet and grepping the output; it composes with
`--repo` (an entry must satisfy both), applies before the total/cleared/actionable/not-evaluable
counts are computed (so the header always agrees with the rows printed under it), and suppresses
the three cross-repo dedup sections below, since those describe the whole corpus rather than a
filtered slice. An invalid regex is a hard error naming the pattern and the regex parse error.
`--audit` switches to a census over both
triage containers instead (see [`--audit` — the `carryover[]`/`reference[]`
census](#--audit--the-carryover-reference-census) below). `--would-block` switches to a
read-only report over `carryover[].blocks[]` edges instead (see [`--would-block` — the honest
blast radius](#--would-block--the-honest-blast-radius) below). `--trajectory` switches to a
read-only weekly outflow table over the archive instead (see [`--trajectory` — the weekly
outflow table](#--trajectory--the-weekly-outflow-table) below). `--dispose` switches to the live
write path this subcommand has — see [`--dispose` — archiving CLEARED
entries](#--dispose--archiving-cleared-entries) below. `--backfill` switches to a one-time,
history-based write path instead — see [`--backfill` — one-time git reconstruction of past
removals](#--backfill--one-time-git-reconstruction-of-past-removals) below.

```bash
mev carryover [--repo <SLUG>] [--include-cross-repo] [--grep <PATTERN>] [--json] [--allow-exec] [--exec-timeout <SECS>] [--audit] [--window <DAYS>] [--dispose] [--backfill] [--dry-run] [--would-block] [--trajectory] [--weeks <DAYS>] [path]
```

| Argument / Flag | Default | Description |
|---|---|---|
| `path` | `.` | Path to search from when locating `brain.toml` (walks up to find it) |
| `--repo <SLUG>` | unset | Restrict the sweep to one repo's `carryover[]` entries **by ownership**, not by which file an entry happens to be stored in — see below. An unknown slug is a hard error naming the valid slugs |
| `--include-cross-repo` | off | Only meaningful together with `--repo`: widens that filter to also match entries scoped `cross_repo: true` (no single owning repo), in addition to the named repo's own entries. Entries owned by a *different* named repo stay excluded — this widens the filter to the unattributable, it does not disable it. Tier-scoped entries stay excluded either way (pinned decision; a separate `--include-tier` is out of scope). Passed without `--repo` this is a misuse: it reports the error and exits non-zero rather than being silently ignored, the same way `--weeks` without `--trajectory` is handled. Cannot be combined with `--audit`, `--trajectory`, `--dispose`, `--backfill`, or `--would-block` |
| `--grep <PATTERN>` | unset | Restrict the sweep to entries whose `slug` or `text` matches this case-insensitive regex. Composes with `--repo` (both narrow; an entry must satisfy both) and with `--json`. Applied before the total/cleared/actionable/not-evaluable counts are computed, so the counts always describe the filtered set. Suppresses the three cross-repo dedup sections (clusters, suggested duplicates, single-repo `finding_id` warnings), since those are statements about the whole corpus. Only applies to the plain per-entry sweep — cannot be combined with `--audit`, `--trajectory`, `--dispose`, `--backfill`, or `--would-block`. An invalid regex exits non-zero naming the pattern and the regex error. A pattern matching nothing exits `0` and says so explicitly, distinguishing "swept, matched nothing" from "nothing to sweep" |
| `--json` | off | Emit the `CarryoverReport` (or, under `--audit`, the `CarryoverAudit`; under `--would-block`, the `WouldBlockReport`; or, under `--trajectory`, the `TrajectoryReport`) as compact JSON instead of the human summary |
| `--allow-exec` | off | Opt in to running `command_exits_zero` predicates. Without it, every such entry reports `not-evaluable` (reason `execution-not-allowed`) and **no command is ever run**. **`--dispose` does not imply this** — passing `--dispose` never turns on command execution, so a `command_exits_zero` entry that is `not-evaluable` for lack of `--allow-exec` is never disposal-eligible either |
| `--exec-timeout <SECS>` | `2` | Wall-clock bound the in-process watchdog enforces on a `command_exits_zero` predicate's child process before killing it. This is a **mev-side** bound — a flag on this command, not a field on the predicate itself; a per-predicate timeout would be an okf-core `ClearsWhenPredicate` schema change and is explicitly out of scope for `MV.16.G`. Ignored without `--allow-exec`. Raising it lets a slower command (e.g. one that touches the network) finish instead of reporting `command-timed-out`; it does not change whether execution is opt-in |
| `--audit` | off | Report a fleet-wide `carryover[]`/`reference[]` census instead of the per-entry sweep — total, per-container and per-kind/per-class counts, typed-predicate coverage, inflow/outflow over `--window` days, and a measured archive-outflow section (MV.16.E). The census figures are composed entirely from the same loaded corpus and `CarryoverReport` the ordinary sweep already produces — no second corpus *walk* — but `--audit` does perform one new read per selected repo, of that repo's `planning/carryover-archive.jsonl`, to produce the archive-outflow section; that read happens only under `--audit`. Shares the exact same `--repo` ownership rule as the ordinary sweep, so `mev carryover --audit --repo X` and `mev carryover --repo X` always agree on which entries are in scope |
| `--window <DAYS>` | `30` | Window, in days, `--audit`'s inflow/outflow figures are measured over. Ignored without `--audit` |
| `--dispose` | off | Move every CLEARED-lane entry out of its owning repo's `state.json` and into that repo's `planning/carryover-archive.jsonl`. The live write path this subcommand has — see below |
| `--backfill` | off | One-time reconstruction of `carryover[]` entries removed from `state.json` **before** `--dispose` existed, recovered from git history and written to that repo's `planning/carryover-archive.jsonl` flagged `reconstructed: true`. Refuses a second run over a populated archive rather than merging. Cannot be combined with `--dispose` — see below |
| `--dry-run` | off | Only meaningful together with `--dispose` or `--backfill`: compute and print the identical plan without writing anything. Passed without either, `mev carryover` reports the misuse and exits non-zero rather than silently ignoring it |
| `--would-block` | off | Report every `carryover[].blocks[]` edge's honest blast radius — owner, edge type, resolved target, the target's live authored status, lane residency, and a verdict — with enforcement off. Read-only, always exits `0`, and cannot be combined with `--dispose`/`--dry-run`/`--audit`. See below |
| `--trajectory` | off | Report the weekly `carryover-archive.jsonl` outflow trajectory instead of the per-entry sweep — one row per ISO week, most recent last, with the observed/reconstructed split and a running cumulative total that must agree with `--audit`'s archive row total. Reads the same archive `--audit` reads and never touches git. Read-only, always exits `0`, and cannot be combined with `--audit`/`--dispose`/`--backfill`/`--would-block`. See below |
| `--weeks <N>` | `8` | Number of week rows `--trajectory` emits, ending with the ISO week containing today. Ignored without `--trajectory` |

Resolves `brain.toml` by walking up from `path`, discovers and loads every repo's
`planning/state.json` (individual load failures are skipped, not fatal), and evaluates every
`carryover[]` entry against the corpus.

**`--repo <SLUG>` selects by ownership (`scope.repo`), never by which file an entry lives in.**
An entry's owner is its own `scope.repo` when set; when `scope.repo` is absent but `scope.tier` or
`scope.cross_repo` is set, the entry has no single owning repo and matches **no** `--repo` filter
for any slug; when `scope` is entirely empty, the entry falls back to the repo of the file it lives
in (the same `own_repo` fallback used for `clears_when` path/command resolution elsewhere). So an
entry that physically lives in repo A's `state.json` but carries `scope.repo: B` is returned by
`--repo B`, **not** by `--repo A` — the file it happens to be stored in is irrelevant to `--repo`.
Before this behaviour was fixed, `--repo` keyed on the file's repo instead, so a cross-repo entry
was invisible to the very repo that owned it and could only be found via the repo it happened to be
filed under. `--audit --repo` applies the identical rule, so the two flags never disagree on which
entries are in scope.

**A bare `--repo <SLUG>` excludes `cross_repo`- and tier-scoped entries — deliberately, but not
harmlessly.** A cross-repo item is no single repo's, so `--repo` correctly does not claim it. But
that also means a repo-filtered view is never the whole picture: entries scoped `cross_repo: true`
or to a `tier` match no `--repo` filter for any slug, and a filtered run says how many such entries
it excluded. Pass **`--include-cross-repo`** alongside `--repo` to widen the view to also include
the `cross_repo: true` entries (tier-scoped entries stay excluded regardless — that is a pinned
decision, not an oversight). `--include-cross-repo` widens the filter to the unattributable; it
never pulls in entries owned by a *different* named repo, and it requires `--repo` — passed alone
it is a misuse, reported and non-zero, the same way `--weeks` requires `--trajectory`.

**The exclusion notice changes wording with the flag, because the remainder changes meaning.**
Without `--include-cross-repo` the excluded set mixes `cross_repo`- and tier-scoped entries and the
flag is the remedy, so the line names both and points at it:

```
filter --repo 'synapse' applied (55 cross-repo/tier entries excluded by this filter;
  add --include-cross-repo to include the cross-repo-scoped ones)
```

With the flag on, every `cross_repo` entry has already been pulled in, so whatever is still excluded
is tier-scoped by construction — and the flag does not widen to those. The line therefore names the
remainder precisely and offers no remedy, rather than advising a flag that is already set:

```
filter --repo 'synapse' applied (2 tier-scoped entries excluded by this filter;
  --include-cross-repo does not widen to tier-scoped entries)
```

The two counts are computed the same way and are not in conflict: 55 drops to 2 because the 53
`cross_repo` entries moved from excluded to included.

**This interacts with `--grep`.** A `--repo`-filtered `--grep` that matches nothing means "no match
in this repo's slice of the corpus" — it is not evidence that no such entry exists fleet-wide. An
entry could be sitting right there, scoped `cross_repo: true`, invisible to the filter. Re-run with
`--include-cross-repo`, or drop `--repo` entirely, before treating a `0 total` as a real negative
(see HQ standing rule 11 on positive controls).

**Without `--dispose`, `mev carryover` writes nothing.** The `cleared` lane is a recommendation
— a human, or a `--dispose` run, acts on it; the plain sweep and `--audit` never delete or
rewrite anything themselves.

#### The three lanes

| Lane | Meaning |
|---|---|
| `cleared` | At least one reference was extracted from the entry and **every** extracted reference is currently satisfied — a recommendation to delete the entry |
| `actionable` | At least one reference was extracted, but **at least one** is unsatisfied — the specific unmet reference(s) are named so a reader can act without re-reading the predicate |
| `not-evaluable` | No reference could be extracted, **or** a reference was extracted but its outcome is unknown rather than a genuine negative. Reason `prose` (`clears_when` is present but is pure prose), `no-closure-verb` (it names a block but never says the block must close), `ambiguous-reference` (a bare block ID matched more than one repo and was dropped, **or** a `file_exists`/`file_contains` path resolved to two different files under the brain root and the owning repo's root), `execution-not-allowed` (a `command_exits_zero` predicate was present but `--allow-exec` was not passed), `command-timed-out` (the child process outran `--exec-timeout` and was killed), `command-spawn-failed` (the child process could not be started at all), `file-unreadable` (a `file_contains` target was missing, larger than 5 MiB, or not valid UTF-8), `pattern-not-literal` (a `file_contains` `pattern` is shaped like a regex and can never match literally), `gate-mention-not-checkable` (it names a validator/gate/CI concept but nothing checkable — no path, no block — could be extracted, flagged as a candidate for a typed `command_exits_zero` predicate), or `no-predicate` (`clears_when` is `None`). **An unknown outcome is never `cleared` and never `actionable`** — a false red only wastes a sweep, but a false clear deletes a finding, so every reason above lands here rather than being folded into a plain unsatisfied reference (`MV.16.G`) |

#### Typed `clears_when` predicates

Alongside prose, `clears_when` may be a typed predicate object (`{"type": "block_closed", ...}`
etc.). All four typed predicate kinds are evaluated:

- **`block_closed { repo, id }`** — satisfied when `"{repo}:{id}"`'s authored status in the
  loaded corpus is exactly `closed`. A `{repo, id}` pair with **no matching node at all** in the
  loaded corpus (a typo'd repo slug or ID) is never satisfied and is reported distinctly from an
  ordinary unmet reference — `unresolvable: {repo}:{id} (not found in loaded corpus)` in the
  human summary, `{"type": "unresolved_block", "key": "..."}` in `--json` — so a data problem
  doesn't read the same as "the block just hasn't closed yet". Unlike the prose grammar, the
  typed form needs no [`CLOSURE_VERBS`](#the-two-evaluable-predicate-classes) gate: it is
  unambiguous by construction.
- **`file_exists { path }`** — satisfied under the same two-root resolution as the prose Class B
  reference below (brain root, then the owning repo's path), **requiring the resolved path to be
  a file** — a directory of the same name no longer satisfies it (`MV.16.G`; previously any
  `.exists()` match did, directory or not). A path present under **both** roots and resolving to
  two different files is reported `not-evaluable` (reason `ambiguous-reference`) rather than
  silently preferring the brain-root candidate; two candidates that resolve to the *same* file
  (e.g. a repo directory reachable through the brain root) are not ambiguous.
- **`file_contains { path, pattern }`** — satisfied when `path` resolves under that same
  two-root strategy (with the same file-not-directory and ambiguity rules as `file_exists` above)
  **and** its contents contain `pattern` as a literal substring (never a regex). The read side and
  the negative-match side are now reported distinctly rather than both folding into `false`:
  - A file that was read successfully with the pattern genuinely absent is `satisfied: false` on
    a `FileContains` reference — an ordinary, actionable unmet reference.
  - A file that is missing, ambiguously resolved, larger than 5 MiB (never read into memory), or
    not valid UTF-8 produces **no** `FileContains` reference at all and forces `not-evaluable`
    (reason `file-unreadable`, or `ambiguous-reference` for the two-root case) — this is evidence
    about the file, never evidence that the pattern is absent.
  - **A `pattern` shaped like a regex is refused rather than matched literally**
    (`MV.16.G`; previously it was matched literally and could therefore never match). The
    evaluator does substring matching only and adds no `regex` dependency — it detects shape
    (`.*`, `.+`, `\d`, `\w`, `\s`, a `[...]` class, `(...|...)` alternation, or a leading `^`/
    trailing `$`) and reports `not-evaluable` (reason `pattern-not-literal`) instead. A bare `.`
    or a lone `^`/`$` inside ordinary prose is deliberately **not** enough to trigger the guard —
    only the composite shapes above do, so a legitimate literal pattern is never mistaken for a
    regex.
- **`command_exits_zero { command }`** — satisfied only when running `sh -c <command>` (cwd: the
  owning repo's path if known, else the brain root) exits with status `0` **and** `--allow-exec`
  was passed. This is the one predicate that executes arbitrary shell from a data file, so it
  carries four deliberate safety properties:
  1. **Opt-in, off by default.** Without `--allow-exec`, `command_exits_zero` entries are never
     run — they report `not-evaluable` with reason `execution-not-allowed` instead. An unrun
     command is unknown, and unknown must never read as `cleared`.
  2. **In-process wall-clock timeout, and it is configurable.** `timeout(1)` does not exist on
     macOS, so the bound is enforced by polling `try_wait` and killing the child on expiry — a bad
     predicate cannot hang a fleet-wide sweep. The bound defaults to 2s and is set per-invocation
     with `--exec-timeout <SECS>` (see the flags table above).
  3. **A timeout is reported distinctly from a failure.** A child process still running when the
     bound elapses produces **no** `CommandExitsZero` reference and forces `not-evaluable`
     (reason `command-timed-out`) — a timeout tells us nothing about what the command would have
     exited, so it is unknown, not failed, and unknown must never read as `cleared`
     (`MV.16.G`; previously a timeout collapsed into the same `satisfied: false` as a genuine
     non-zero exit and was reported `actionable`, indistinguishable from a real failure). A
     command that exits non-zero *within* the bound is still `satisfied: false` on a
     `CommandExitsZero` reference and lands `actionable` as before.
  4. **A spawn failure is reported distinctly too.** A child process that could not be started at
     all (e.g. `sh` not on `PATH`) produces no reference and forces `not-evaluable` (reason
     `command-spawn-failed`) — evidence about the environment the sweep ran in, never evidence
     about the predicate's subject.

#### The two evaluable prose predicate classes

Only two classes of prose `clears_when` are ever machine-evaluated; anything else falls into
`not-evaluable` rather than being guessed at:

- **Block references — from `clears_when` only.** Block IDs matched in the prose by a strict
  grammar (`[A-Z]{2,3}\.(?:\d+\.[A-Z0-9]+|ticket\.[a-z0-9][a-z0-9-]*|chore\.[a-z0-9][a-z0-9-]*)`).
  A match is kept only when **both** hold:
  1. The predicate contains a word-bounded **closure verb** — one of `land` · `lands` ·
     `landed` · `landing` · `ship` · `ships` · `shipped` · `shipping` · `merge` · `merges` ·
     `merged` · `closes` · `closed`. A predicate that names a block without one is reported
     `not-evaluable` with reason `no-closure-verb`.
  2. The token resolves to exactly one node in the loaded corpus (preferring the carryover's
     own scope repo when the bare ID is ambiguous); an ID resolving to nodes in more than one
     repo is dropped and the entry reported `not-evaluable` with reason `ambiguous-reference`
     rather than guessed at. An unresolvable token is simply not a block reference and is
     discarded silently.

  A block reference is satisfied when its node's authored status is `closed`.

  **`related[]` is not consulted.** The schema documents it as *optional related edges* — a
  "see also", not a clearing condition. A carryover merely related to block X does not clear
  when X closes.

> **Why both gates exist.** Verified against the live corpus 2026-08-03:
> `core:ba-0-a-id-collision` reads *"one of the two `BA.0.A` blocks is renamed and Phase 0 is
> backfilled"*, and `BA.0.A` **is** `closed`. Without the closure-verb gate the sweep
> recommended deleting a live, unresolved `known_issue`. A false `cleared` is the only verdict
> here that destroys durable knowledge.
- **Path assertion references** — extracted only when the `clears_when` text contains a
  word-bounded entry from a bounded, documented verb vocabulary — the path analogue of the
  closure-verb gate above. A path named with no assertion verb at all is a *subject*, not a
  *condition*, and nothing is extracted for it (`not-evaluable`, reason `prose`).
  - **Presence verbs** — `exists` · `created` · `added` · `written` · `present` · `corrected` ·
    `fixed`. Whitespace-delimited tokens containing `/` and ending in one of
    `.md .rs .py .sh .ts .tsx .json .toml` are resolved against the brain root and against the
    owning repo's `repo_path`; satisfied when either resolves to an existing file. The
    `corrected`/`fixed` verbs pair with an already-checkable file this way rather than attempting
    to parse what "corrected" means — a predicate like *"X is corrected"* with no named file/block
    stays `not-evaluable`.
  - **Absence verbs** — `removed` · `deleted` · `gone`. Same path resolution, but satisfied when
    the path does **not** resolve to an existing file — the inverse polarity, reported as a
    distinct `path_absent` reference (`{"type": "path_absent", "path": "...", "satisfied": ...}`
    in `--json`) so "the path exists" is never conflated with "the path is gone".
  - When a predicate names a validator/gate/CI concept (`validator` · `gate` · `lint` · `linter` ·
    `harness` · `pipeline` · `suite` · `ci`) but nothing checkable was extracted from it, it is
    reported `not-evaluable` with reason `gate-mention-not-checkable` rather than plain `prose` —
    a hint that it is a candidate for a typed `command_exits_zero` predicate. Nothing derives a
    command from this prose and runs it automatically; that stays explicitly out of scope.

**All extracted references are combined conjunctively (AND), even when the prose says "or".**
This is a deliberate, safe-failure-direction bias: it can mis-report a genuinely-cleared
`or`-predicate as `actionable`, but it can never mis-report an unmet dependency as `cleared`.
Disjunction parsing is out of scope.

Every reported entry also carries its repo, slug, kind, `age_days`, and a `stale` flag derived
from the existing `carryover_stale_age` helper (honouring `reviewed` / `snoozed_until`) — no
staleness logic is reimplemented here. As of `MV.ticket.carryover-triage-ranking`, each entry
additionally passes through its authored `priority` (0..=3, absent when not set), `finding_id`,
and `blocks[]` (the `BlockedBy` edges the entry gates) verbatim — the same fields the Attention
board's carryover triage lanes are ranked on (see [Carryover triage lanes](state.md#carryover-triage-lanes)
above and [carryover-contract.md](../carryover-contract.md) for the full, versioned wire
shape). `mev carryover`/`mev carryover --json` itself still sorts and reports the three
`clears_when` lanes (`cleared`/`actionable`/`not-evaluable`) below — those are an orthogonal
question from the four triage lanes and are unaffected by this block.

#### Cross-repo dedup: clusters, suggestions, and the typo guard

`MV.ticket.carryover-dedup-clusters` adds a second, orthogonal pass over the same loaded
`entries` — no new file reads, no second discovery walk. It answers "is this the same finding
filed more than once?" using the free-form, authored `finding_id: Option<String>` field.

- **`clusters`** (`CarryoverReport.clusters`, human section `CLUSTERS`) — every entry sharing a
  non-empty `finding_id`, grouped exactly one cluster per distinct id string. Grouping is exact
  (no case-folding, no fuzzy join): `finding_id` is hand-written by a human, so the human is the
  identity authority, not the tool. Two or more entries in the *same* repo may legitimately share
  one `finding_id` (many-to-one) — they still appear as distinct members, never collapsed.
  **Per-repo priority divergence is shown side by side and is never reconciled.** A claim can be
  genuinely P0 in one repo and genuinely P2 in another (the measured case: a `nextest` claim is
  P0 in `okf-core`, where the hook does not fire, and P2 in `mev`, where it works as documented) —
  dedup merges the *claim*, never the *priority*. No merged/effective/max/min priority field
  exists anywhere in the shape, and no diagnostic is emitted merely because priorities diverge.
- **`suggestions`** (`CarryoverReport.suggestions`, human section `SUGGESTED DUPLICATES —
  UNCONFIRMED`) — candidate duplicate pairs among entries that carry **no** `finding_id`, from a
  crude token-overlap pass over `slug` + `text` (stopwords removed, tokens under 3 chars
  dropped). A pair is suggested when `jaccard >= 0.18` **or** `overlap_coefficient >= 0.34` —
  both operator-measured against the live corpus, in `DEDUP_JACCARD_MIN` /
  `DEDUP_OVERLAP_MIN`. **Suggestions are never auto-applied.** They do not mutate `finding_id`
  and are not written to any file; a human confirms a suggested pair by hand-authoring the same
  `finding_id` string onto both entries' `planning/state.json`. The heading itself carries
  UNCONFIRMED, not only a trailing note, since a heading is what survives a skim.
- **`single_repo_finding_ids`** (human section `SINGLE-REPO finding_id WARNINGS`) — the sorted
  list of `finding_id` values whose cluster spans exactly one repo. A `finding_id` is meant to
  link the *same* finding **across** repos; one that never left a single repo usually means the
  id was mistyped somewhere and silently failed to group with its intended match — the same
  "field nothing validates" defect class this feature exists to close.

All three sections are omitted from the human summary when empty, matching the existing lane
behaviour, and none of them affects the exit code.

#### `--audit` — the `carryover[]`/`reference[]` census

`mev carryover --audit` (`MV.ticket.reference-container-validation` task 4) answers "what does
the fleet's triage material actually look like", as opposed to the per-entry sweep above, which
answers "what should a human act on right now". Its census figures (totals, per-kind, per-class,
typed-predicate coverage, clear rate, inflow/outflow) are composed entirely from the same loaded
corpus (`files`) and `CarryoverReport` the ordinary sweep already produced — no second corpus
*walk*. `--audit` itself does perform one new filesystem read beyond that: for each selected
repo it reads `planning/carryover-archive.jsonl` (MV.16.E) to produce the outflow-by-archive
section below. That read happens only on the `--audit` path — the plain sweep, `--dispose`,
`--backfill` and `--would-block` never touch the archive. Like the plain sweep, the whole command
is still **read-only**: the audit recommends; `--dispose` is the only invocation of this
subcommand that writes anything.

| Figure | Meaning |
|---|---|
| `total` / `carryover_count` / `reference_count` | Fleet-wide entry count, and the split across the two containers |
| per-kind (`carryover[]`) | `carryover[]` entries grouped by `kind` — includes legacy `constraint`/`known_issue` wherever they still appear, since D72's narrowing didn't rewrite any data |
| per-class (`reference[]`) | `reference[]` entries grouped by `class` (`trap`/`invariant`/`lesson`/`deliberate`, plus any not-yet-valid value present in the corpus) |
| typed-predicate coverage | How many `carryover[]` entries carry a typed `clears_when` predicate (`block_closed`/`file_exists`/`file_contains`/`command_exits_zero`) rather than free prose or no predicate at all |
| clear rate — deletions only | `cleared_total / clearable_total` — **scoped to `carryover[]` only.** `reference[]` entries have no `clears_when` and are structurally never clearable, so they are excluded from the denominator by construction, not by a filter: a raw per-repo rate would punish reference-heavy repos for behaving correctly (measured on the live corpus: `bastiel` 11%, `okf-core` 0/14 — composition, not discipline). Labelled "deletions only" because it counts entries that are still `carryover[]` rows sitting in the CLEARED lane — it cannot see a disposal, which *removes* the entry — so the archive-outflow section below is the figure that actually measures disposition |
| inflow / outflow (proxy) | Entries whose `created` date falls within `--window` days of today (inflow), and `Cleared`-lane entries whose staleness anchor (`max(created, reviewed)`) falls within the window (outflow) — a proxy for "recently became safe to delete", since no container records an actual deletion timestamp. Superseded as a disposition measure by the archive outflow below, which reads real disposal records instead of inferring from what is still present |

**Outflow (archive) — measured dispositions, split observed / reconstructed (MV.16.E).** Beneath
the inflow/outflow proxy line, `--audit` prints a second section built from
`planning/carryover-archive.jsonl` — the append-only record `--dispose` and `--backfill` both
write, one `CarryoverArchiveRow` (okf-core) per line, derived per repo with the same
`archive_path_for` helper the rest of the archive tooling uses. Unlike the proxy line above, this
section counts entries that actually left `carryover[]`, however they left — a disposal is
visible here even though it has already vanished from the corpus the plain census reads.

- **Per-`reason` counts**, keyed on `DisposalReason` (`cleared`/`superseded`/`promoted`/
  `withdrawn`), each split into two columns:
  - `observed` — rows a live `--dispose` run wrote (`reconstructed: false`, the default when the
    key is absent).
  - `reconstructed` — rows `--backfill`'s one-time git reconstruction wrote (`reconstructed:
    true`).
- **Why the split is never blended.** `--backfill`'s reconstructed rows carry weaker, inferred
  evidence — at least one reconstructed removal in the live corpus is a relocation to another
  repo, not a disposal at all — and a downstream published post quotes these figures. Blending the
  two columns into one number would let inferred, sometimes-wrong data inflate a headline that
  gets published and cannot be walked back.
- **Missing or empty archive is the normal case, not an error.** `--backfill` and `--dispose` are
  both applied per repo on demand, so a repo (or the whole fleet, until either has run once
  against it) can legitimately have no `carryover-archive.jsonl` yet. `--audit` reports this
  silently — zero rows, no diagnostic — and still exits `0`; it never treats an absent archive as
  a failure.
- **A malformed archive line is named, not swallowed.** A line that fails to parse as
  `CarryoverArchiveRow` is reported as `<path>:<1-based line number>` (up to 5 shown) and skipped;
  the surrounding valid rows are still counted, and the run still exits `0`.
- **Same `--repo` scoping as the rest of the audit.** `--audit --repo X` reads the archive only
  for the repos its `carryover[]`/`reference[]` census already selected, so the two halves of the
  report always agree on which repos are in scope.
- The full `CarryoverArchiveRow` schema (the four extra fields an archive row carries beyond a
  plain `Carryover`) is documented in [carryover-contract.md](../carryover-contract.md); this
  section only covers how `--audit` reads and reports it.

#### Exit codes

| Code | Meaning |
|---|---|
| `0` | Sweep (or, under `--audit`, the census) completed successfully, regardless of how many entries land in any lane |
| `1` | `brain.toml` not found/unreadable, an unknown `--repo` slug, an invalid `--grep` regex (names the pattern and the regex error), or a serialization error under `--json` |

**Examples:**

```bash
# Human, lane-grouped summary of the whole fleet
mev carryover

# Machine-readable JSON envelope
mev carryover --json

# Restrict to one repo
mev carryover --repo mev

# Find one known entry by slug or text, without dumping the fleet
mev carryover --grep synapse-rename

# --grep composes with --repo and --json
mev carryover --repo orchestrator --grep 'synapse-rename' --json

# From an explicit brain root
mev carryover ~/Dev/agentic-portfolio

# Opt in to running command_exits_zero predicates
mev carryover --allow-exec

# Fleet-wide carryover[]/reference[] census, default 30-day window
mev carryover --audit

# Census over a 90-day inflow/outflow window, as JSON
mev carryover --audit --window 90 --json

# Move every CLEARED entry to the archive (real write)
mev carryover --dispose

# Inspect what a real --dispose run would move, without writing anything
mev carryover --dispose --dry-run

# Restrict disposal to one repo
mev carryover --dispose --repo mev

# Preview the blast radius of every carryover[].blocks[] edge (read-only)
mev carryover --would-block

# Same, as JSON, restricted to one repo
mev carryover --would-block --json --repo mev

# Weekly outflow trajectory, default 8 weeks
mev carryover --trajectory

# Last 4 weeks only, as JSON
mev carryover --trajectory --weeks 4 --json

# Restricted to one repo, matching that repo's --audit scope
mev carryover --trajectory --repo mev
```

#### `--trajectory` — the weekly outflow table

`mev carryover --trajectory` (`MV.16.F`) answers "what does the fleet's carryover outflow look
like over time", decomposed by week, as opposed to `--audit`'s single archive-outflow total. It
reads `planning/carryover-archive.jsonl` through the exact same reader `--audit` uses — MV.16.E's
`read_archive_outflow` machinery, refactored to share a `collect_archive_rows` helper with the new
`build_trajectory` — so there is one archive reader in this codebase, not two. **It never reads
git.** Git was MV.16.B's one-time reconstruction pass that populated the archive in the first
place; a second git reader here would re-derive the same numbers a different way and could drift
from `--audit` the moment a disposal happened outside whatever range it walked.

- **Bucketing.** Rows are bucketed by the ISO week (`YYYY-Www`) of their `disposed_at` date.
  `--trajectory` emits exactly `--weeks` rows, most recent last, ending with the week containing
  today — **including weeks with zero disposals**. A week that silently drops off the table because
  nothing happened in it would misrepresent the trajectory as sparser than it is.
- **Columns.** Each row carries `observed`, `reconstructed`, the row `total` (their sum), and a
  running `cumulative` total. Reconstructed rows (`--backfill`'s git-derived rows, flagged
  `reconstructed: true`) are always shown in their own column, never merged into `observed` — the
  same one-way-publication reason `--audit`'s archive-outflow section keeps the two split (weaker,
  inferred evidence should never inflate a number that gets published).
- **`earlier (before window)`.** Archive rows dated before the first emitted week are not dropped;
  they are counted in an explicit `earlier (before window): N` line printed above the table and
  folded into the first row's `cumulative`, so a narrow `--weeks` window can never silently disagree
  with the fleet-wide total.
- **`undated` rows.** A row whose `disposed_at` fails to parse is excluded from every week bucket
  (it cannot be assigned an ISO week) but still counts toward `rows_total` — printed as a dedicated
  line when `undated > 0`, since silently bucketing an unparseable date would put a wrong number in
  a published table.
- **The coherence guarantee.** For a window that covers the whole archive, the last row's
  `cumulative` equals `--audit`'s `archive_outflow.rows_total` for the same `--repo` scope — this is
  the block's headline criterion, and `tests/it/brain_carryover_trajectory.rs` asserts it directly,
  including that both numbers move together when one more row is disposed into the fixture.
- **Mutual exclusions.** `--trajectory` cannot be combined with `--audit`, `--dispose`,
  `--backfill`, or `--would-block` — each combination is reported as a misuse naming both flags and
  exits non-zero, mirroring the existing `--would-block` misuse check. `--weeks` is ignored when
  passed without `--trajectory`, the same way `--window` is ignored without `--audit`.
- **`--repo` scoping.** `--trajectory --repo X` reads the archive for exactly the repos
  `--audit --repo X` reads, and no others, so the two commands' totals are always comparable.
- **Missing or empty archive.** Same as `--audit`: no `carryover-archive.jsonl` yet is the normal
  case, not an error. `--trajectory` prints the single no-archive summary line (`0 archive row(s)
  over 0 archive(s)`) and nothing else, and exits `0`.
- **`--json`** serializes the `TrajectoryReport` struct directly — the same figures the human table
  shows, so the published build-log post (`business:BZ.6.C`) can generate from data instead of
  scraping the table.

#### `--dispose` — archiving CLEARED entries

`mev carryover --dispose` (`MV.ticket.carryover-dispose`) is the missing outflow half of the
sweep: without it, nothing in the fleet ever acts on a `cleared` verdict, so resolved entries
accumulate in `carryover[]` forever. It re-runs the exact same sweep as the plain command, then
acts on the `cleared` lane only:

- **A disposal is a MOVE, never a delete.** Each CLEARED entry is removed from its owning
  repo's `planning/state.json` `carryover[]` array and appended, verbatim plus four extra
  fields, to that repo's `planning/carryover-archive.jsonl` as a `CarryoverArchiveRow`
  (okf-core `OK.4.A`, via `serde(flatten)`):
  - `disposed_at` — the date of the run
  - `reason` — always `"cleared"` for this write path (the other `DisposalReason` values are
    for other, not-yet-built disposal routes)
  - `reconstructed` — always `false` (this row was produced live from a real sweep, not
    rebuilt from git history)
  - `evidence` — the clearing predicate that landed the entry in CLEARED, so the archive line
    is self-explaining without cross-referencing the original `clears_when`. For a
    `command_exits_zero` clearance this now also names the `--exec-timeout` bound that was in
    force (`command \`X\` exited 0 (bound Ns)`) rather than a bare `command X exited 0`
    (`MV.16.G`) — a disposal is otherwise unfalsifiable after the fact, since the archived row
    alone couldn't say whether the command that cleared it ran under a 2s bound or a 200s one
  - Carryover entries are kept as data for history and analysis — the archive file is
    append-only and nothing is ever discarded.
- **Both writes land together.** The `state.json` removal and the `carryover-archive.jsonl`
  append for a given repo are staged and committed as one atomic step; if either side cannot be
  completed, neither is applied and the repo is reported failed rather than left with an entry
  removed-but-unarchived or a malformed `state.json`. `mev carryover --dispose` prints the exact
  `git commit -o <pathspec>` covering both files for every repo it wrote, so the operator commits
  both halves together.
- **A repo whose sweep failed to evaluate is skipped, not silently treated as clean.** If a
  repo's `state.json` fails to load or parse, it is named in the output and both of its files
  are left untouched; sibling repos in the same run are still disposed normally.
- **`--dispose` never implies `--allow-exec`.** A `command_exits_zero` predicate that is
  `not-evaluable` for lack of `--allow-exec` stays `not-evaluable` and is therefore never
  disposal-eligible — only an entry whose command was actually run (with `--allow-exec` passed)
  and exited `0` can land in CLEARED here.
- **Full text before removal.** Each disposed entry's complete text is printed before it is
  moved, so a run whose output has scrolled past the terminal buffer is still fully readable.
- **A per-repo summary line is always printed**, including repos where nothing moved
  (`0 disposed`) and repos that were skipped — so a no-op run is distinguishable from a run that
  never reached that repo.
- **Byte-faithful `state.json` round-trip.** The rewritten file differs from the original by
  exactly the removed array elements — same indentation, same key order, non-ASCII (em dashes,
  etc.) left unescaped, trailing newline preserved.
- **`--dispose --dry-run` is the same code path with both writes suppressed** — identical
  disposal list, identical per-repo summaries and commit-pathspec preview, zero bytes written.
  There is no separate dry-run implementation to drift from the real one.

Exit code `0` on a successful run, including one where some repos were skipped (reported, not
fatal); non-zero if `--dry-run` is passed without `--dispose` or `--backfill`, or on the same
`brain.toml`/`--repo`/serialization failures the plain sweep can hit.

#### `--backfill` — one-time git reconstruction of past removals

`mev carryover --backfill` (`MV.16.B`) is the missing history half of `--dispose`: `--dispose`
only records outflow from the moment it shipped forward, so every `carryover[]` entry removed
from a `state.json` *before* that — by hand, by a script, by any commit at all — left no trace in
any archive. This pass walks git history instead of the live sweep and reconstructs those past
removals into the same `CarryoverArchiveRow` shape `--dispose` writes, so `--audit`'s outflow
accounting has something to read for the period before `--dispose` existed.

- **It is a ONE-TIME pass, not a recurring one.** A run over a repo whose archive already has
  rows refuses outright rather than merging or diffing against what is there — see the refusal
  rule below. There is no "catch up the archive again" mode; once a repo's archive is
  backfilled, later removals are recorded by real, live `--dispose` runs only.
- **A "removal" is a diff, per commit,** between a `carryover[]` entry present in that commit's
  parent and absent in the commit itself. A commit that only adds or only edits an entry
  produces no row.
- **It writes ONLY `carryover-archive.jsonl`, never `state.json`.** The entries this pass
  recovers are already gone from `state.json` — that absence is the premise of the whole block —
  so there is nothing for it to remove there. Contrast `--dispose`, whose defining act is
  removing the entry from `state.json` at the same time it archives it.
- **Every emitted row carries `reconstructed: true`.** This is the field a consumer checks to
  tell a row this pass wrote from one a live `--dispose` run wrote — `MV.16.E`'s outflow
  breakdown splits its counts on exactly this field and must never blend the two, because a
  reconstructed row is not a closure event in the same sense a live disposal is (at least one
  removing commit in the corpus is a relocation to another repo, and at least one is a generator
  run — neither is a disposal in any meaningful sense; the honest claim is that this output is
  raw).
- **The reason-derivation rule.** `okf_core::DisposalReason` is a closed four-value enum —
  `cleared | superseded | promoted | withdrawn` — with no `unknown` member, and this pass does
  not add one (that type lives in okf-core, another repo; a fifth member to describe our own
  ignorance would duplicate what `evidence` and `reconstructed` already carry). The removing
  commit's subject is matched, case-insensitively, against a small keyword set per reason
  (wording about clearing/resolving → `cleared`; superseding/replacing → `superseded`;
  promoting → `promoted`). **When the subject names none of these, the reason defaults to
  `withdrawn`** — "retired without being resolved" is the only member that asserts nothing
  unevidenced — and the row's `evidence` string says explicitly that the reason was not
  attributable from the commit subject, rather than fabricating a more specific one.
- **The embedded entry is the parent commit's value, verbatim.** `CarryoverArchiveRow` flattens
  `Carryover`, which itself ends in a catch-all `extra` map (okf-core's own doc comment flags
  this nested-flatten hazard) — the entry is deserialized from the removing commit's PARENT blob
  and carried through unchanged, including any key `Carryover` does not model. It is never
  re-synthesized from the fields this pass happens to care about, which would silently drop
  every unmodeled key.
- **`evidence` always names the removing commit** as `<short-sha> <subject>`, plus the
  not-attributable note when the reason was defaulted.
- **Idempotent by refusal, never by merge.** Before writing anything, every repo's existing
  archive is read and indexed by `(slug, disposed_at)` — the identity `okf_core::AmendsRef`
  already establishes as an archive row's unique key. If any planned row collides with an
  existing one, the ENTIRE run aborts before a single byte is written anywhere, naming the
  colliding `(slug, disposed_at)` pair, and exits non-zero. It does not skip the collision and
  write the rest — a partial backfill is harder to reason about than none.
- **Atomic per-repo write, revert on failure.** Mirrors `--dispose`'s own discipline: each
  repo's archive file's original bytes are read first, and on any write error the file is
  reverted to exactly those bytes before the error is reported; sibling repos in the same run
  are unaffected.
- **`--backfill --dry-run` is the same code path with the write suppressed** — identical plan,
  identical per-repo summary and commit-pathspec preview, zero bytes written.
- **Prints the exact `git commit -o <pathspec>`** covering every archive file it wrote, because
  every `planning/` is a symlink into the one HQ git repo where `git add -A` is banned (Standing
  Rule 10).
- **Composes with `--repo <slug>`**, restricting both the history walk and the writes to that
  repo's state file, with the same ownership semantics `--dispose --repo` already has.
- **Deliberately not gated by `harness.json`.** It is a one-time command; running a full history
  walk on every push would be pure overhead for a check that, after the first successful run,
  can only ever refuse.

Exit code `0` on a successful run (including a `--dry-run`); non-zero on a collision against a
populated archive, a write failure, or the same `brain.toml`/`--repo` failures the plain sweep
can hit.

```bash
# Preview the full reconstruction plan without writing anything
mev carryover --backfill --dry-run

# Run the one-time backfill for the whole fleet
mev carryover --backfill

# Restrict to one repo
mev carryover --backfill --repo mev
```

#### `--would-block` — the honest blast radius

`mev carryover --would-block` (`MV.16.A`) answers, before any enforcement ships, the one question
nobody could answer before this block: **if `carryover[].blocks[]` actually gated work today,
what would stop?** Enforcement shipped in `MV.16.C`, gated behind `enforce_blocks` and a per-repo
cap and **off by default** — so with the default configuration `blocks[]` still only propagates
priority, and the only thing that actually holds a block back is a `depends_on {type: "block"}`
edge. This report previews exactly what turning `enforce_blocks` on would hold, and stays
read-only whether enforcement is on or off. It reports the current state in its header as
`enforcement: ON (cap N/repo)` or `enforcement: OFF`.

It walks every `carryover[].blocks[]` edge in the swept corpus (the identical corpus the plain
sweep and `--audit` already load — no second discovery walk) and emits one row per edge:

| Column | Meaning |
|---|---|
| owner | The `carryover[]` entry that carries the edge, as `{repo}:{slug}` |
| edge type | `block` / `external` / `operator` / `approval` — the `BlockedBy` edge's own type |
| target | The resolved `{repo}:{id}` the edge points at, or `-` for a non-`block` edge, which has no node target |
| status | The target's live authored status (e.g. `open`, `closed`, `wontfix`), or `-` when the edge has no node target |
| lane? | `true`/`false` — whether the target appears in any `lane-<name>.json` record discovered by `discover_lane_files`. **A separate axis from status**: an `open` target in no lane and an `open` target someone is actively driving are both `blocking`, and this column is what tells them apart. Always `false` for a non-`block` edge |
| verdict | `blocking` / `closed` / `wontfix` / `unresolvable` / `no-node-target` — see below |
| lanes | The lane identifier(s) (`{roadmap}/lane-{lane}.json`) the target was found in, or `-` when not lane-resident |

**The verdict, all five cases explicit — never collapsed into a bare `!= closed` test:**

| Verdict | Meaning | Counted as blocking? |
|---|---|---|
| `blocking` | A `block` edge whose target resolved and whose authored status is neither `closed` nor `wontfix` | yes |
| `closed` | A `block` edge whose target resolved with status `closed` — gates nothing | no |
| `wontfix` | A `block` edge whose target resolved with status `wontfix` — gates nothing. Not in `sequence.md`'s original cut; measured live in the corpus on `JF.2.A` and handled explicitly rather than falling through a `!= closed` test | no |
| `unresolvable` | A `block` edge whose target does not resolve to any node in the loaded corpus (a typo'd repo/id) — a data defect, not a live block. Reported so the defect is visible, but never counted toward the blast radius, since counting it would inflate the number with typos | no |
| `no-node-target` | An `external` / `operator` / `approval` edge — these have no node target by construction and are reported as rows, never silently dropped, so the report never looks complete while hiding edges | no |

The report's summary line totals every edge and breaks the non-blocking count down by reason
(`closed` / `wontfix` / `unresolvable` / `no-node-target`), so a reader sees at a glance how many
edges were excluded and why, never just a bare headline number.

**Shares its resolution rules with `unmet_carryover_block_keys` — never a second copy.** Both are
built from the same edge-classification core in `src/brain/carryover.rs`, so a dry-run that
disagreed with the predicate `MV.16.C`'s enforcement will actually gate on would be worse than no
dry-run at all. The only deliberate divergence is the `wontfix`/`unresolvable` carve-out this
report adds — everything else agrees edge-for-edge.

**Composes with `--repo <slug>`**, restricting the rows to the named repo's owning entries, the
same ownership rule the plain sweep uses.

**Never writes, never gates.** `--would-block` opens no file handle for writing on this path,
leaves every `state.json` and `carryover-archive.jsonl` byte-identical, and is **not** part of
`planning/harness.json` — nothing in this fleet's push gate depends on what it finds. Enforcement itself
is `MV.16.C`'s, behind the `enforce_blocks` flag and a per-repo cap and off by default; this flag
only ever previews, and never applies a gate regardless of how `enforce_blocks` is set.

**Cannot be combined with `--dispose`, `--dry-run`, or `--audit`** — pass it alone (optionally
with `--repo`/`--json`); combining reports the misuse and exits non-zero rather than silently
picking one.

Exit code `0` regardless of what it finds — a non-zero blocking count is a finding, not a failure;
that is the whole point of shipping this before enforcement exists. Non-zero only on the same
`brain.toml`/`--repo`/serialization failures the plain sweep can hit, or on an incoherent flag
combination as above.

#### `[carryover]` — turning `blocks[]` edges into real gates (`MV.16.C`)

`--would-block` above only previews. `MV.16.C` is the enforcement it previews: a `brain.toml`
`[carryover]` section that, when turned on, makes a `carryover[].blocks[]` edge actually hold the
block it names, in the same derivation every generated board and both validators read.

```toml
[carryover]
enforce_blocks = true   # default: false
max_gates_per_repo = 10 # default: 10
```

| Key | Default | Meaning |
|---|---|---|
| `enforce_blocks` | `false` | Master switch. `false` (or an absent `[carryover]` table entirely) reproduces today's behaviour exactly — no gates, no change to any derived surface. `true` turns every qualifying `carryover[].blocks[]` edge into a real hold |
| `max_gates_per_repo` | `10` | Per-repo cap on how many gates `enforce_blocks = true` may apply. Exceeding it never silently truncates — see **The cap** below |

**An absent `[carryover]` table is not a degraded mode — it is identical to `enforce_blocks =
false`.** Shipping this section and flipping it on for the real corpus are different acts: this
block ships the mechanism only. Turning it on for the fleet is HQ's `HQ.7.C`, gated behind an
install-closure edge and an operator approval, because an older installed `mev` binary silently
ignores an unknown `enforce_blocks` key (`BrainConfig` deliberately has no `deny_unknown_fields` —
closing that is a separate, fleet-wide change, out of scope here) and the Mini's nightly
`routine.sh` would regenerate boards with enforcement silently absent on a stale install — a flap
with no error anywhere.

**Enforcement's home is the block-level startability derivation (`derive_focus` / `ready_order` in
`src/brain/state.rs`), never `compute_frontier`.** `compute_frontier` is lane-head-scoped and
represents a closed block by its *absence*, so a gated block that sits in no lane would be held
invisibly there — measured on the live corpus, most of the block edges that exist point at
targets in no lane. `derive_focus` is what `emit-block-graph`, `validate-brain --state`, and every
generated board actually read; `compute_frontier` and the availability derivation are downstream
**consumers** of its held-ness, not a second place that recomputes it. A gated block reported held
carries a reason naming the owning carryover entry's slug, so a board says *why* a block is held,
not merely that it is — and the flag off, `derive_focus`/`ready_order`/the frontier/availability
output is byte-identical to a build with no `[carryover]` table at all.

**Only a live `block` edge whose target resolves and is neither `closed` nor `wontfix` gates.**
This reuses the exact same edge-classification predicate `--would-block` reports with (`src/brain/
carryover.rs`) — the enforcement and its own dry-run are built on one shared function, not two, so
they cannot drift apart. `external`/`operator`/`approval` edges never gate (they have no block
target to hold), and an edge that fails to resolve to any node in the loaded corpus (a typo'd
repo/id) gates nothing either — a data defect must never hold real work.

**Three independent ways out of a wedge — the risk this section exists to bound is one bad edge
holding a lane nobody can unstick, and each of these breaks that on its own, without touching the
other two:**

1. **The flag.** Set `enforce_blocks = false` (or delete the `[carryover]` table) and every gate
   this section applies disappears fleet-wide, instantly.
2. **The per-repo cap.** `max_gates_per_repo` bounds how many gates any one repo's edges may apply
   at once, regardless of how many qualifying edges exist.
3. **The per-entry `enforce: false` opt-out.** Any single `carryover[]` entry can opt its own
   `blocks[]` edges out without touching the flag or the cap. This is okf-core's rule
   (`core/okf-core/src/state.rs`) — `mev` consumes the edges okf-core already suppressed rather
   than re-implementing the check; `None` and `Some(true)` both enforce, only an explicit
   `enforce: false` opts out.

All three are covered by `tests/brain_carryover_enforcement.rs`.

**The cap is reported when exceeded, never silently applied.** If a repo's qualifying edges exceed
`max_gates_per_repo`, enforcement applies none of the excess — only the first `max_gates_per_repo`
gates take effect — and `mev carryover --would-block` prints `cap exceeded — {repo}: N of M gates
applied` for that repo. Silent truncation would read as "enforcement is on and nothing is held",
which is indistinguishable from the flag being off; this section never lets that ambiguity stand.

**`--would-block` reports the live enforcement state**, so the same dry-run output can no longer
mean two different things depending on config nobody can see from the report:

```bash
mev carryover --would-block
# enforcement: OFF                              (no [carryover] table, or enforce_blocks = false)
# enforcement: ON (cap 10/repo)                  (enforce_blocks = true, default cap)
# enforcement: ON (cap 2/repo)
# cap exceeded — mev: 2 of 3 gates applied
```

**`blocked` is derived, never authored.** No code path in this section ever writes a `blocked`
value onto a `tracks[]` block's `status`, and never synthesizes a `depends_on` entry to represent
a carryover gate. Doing either would be `E_BLOCK_BAD_STATUS`-class misuse and would turn a
derived fact into a permanent, hand-editable one — `tests/brain_carryover_enforcement.rs` sweeps
the fixture corpus after every derive to assert this never happens.

---

### `backlog [--repo <slug>] [--lane <lane>] [--json] [--allow-exec] [path]`

The read-only sweep `backlog[]` never had. It mirrors [`carryover`](#commands) deliberately — same
flags, same four predicate kinds, same lane vocabulary — pointed at the second container, so there
is one idiom to learn rather than two.

**It cannot write anything.** There is no `--dispose` and no mutation mode anywhere in the verb,
by construction. `mev carryover --dispose` destroyed 12 live entries on 2026-09-02 by mining
free-prose predicates and evaluating them as if typed; this verb was specified without a disposal
mode for that reason.

Unlike `carryover`'s, `backlog[]`'s `clears_when` / `ready_when` are **typed fields on the node** —
no prose extraction — evaluated with the same four kinds: `block_closed`, `file_exists`,
`file_contains`, and `command_exits_zero`, which is **never executed** unless `--allow-exec` is
passed.

| Lane | Means |
|---|---|
| `CLEARED` | `clears_when` is satisfied — the idea is dead. **Read this as "predicate satisfied — verify before acting", not as a verdict.** |
| `READY` | `ready_when` is satisfied — promote it. |
| `WAITING` | `ready_when` is evaluable but not yet satisfied. |
| `AGING` | Nothing evaluable, and the entry is older than `brain.toml`'s `[attention] backlog_days`. |
| `NOT-EVALUABLE` | A prose predicate, a `command_exits_zero` without `--allow-exec`, or a predicate-free entry too young to age. |

| Flag | Effect |
|---|---|
| `--repo <slug>` | Restrict the sweep to one repo's entries. |
| `--lane <lane>` | Restrict the **printed rows** to one lane. The header totals still describe the whole sweep — a filtered view is not a smaller corpus. |
| `--json` | Emit the `BacklogReport` as compact JSON. |
| `--allow-exec` | Opt in to running `command_exits_zero` predicates. Without it they report `NOT-EVALUABLE` and are **provably not run**. |

Exit codes: **0** whenever the sweep completes, however the entries land; **1** only if `brain.toml`
is missing or unreadable, `--repo` names an unknown slug, or `--json` fails to serialize.

**Every lane reading zero except `AGING` and `NOT-EVALUABLE` is the expected result today**, and is
not a broken sweep. Nothing authors the two predicates yet — neither `/backlog-ticket` nor
`/capture` sets them — so no entry can be `CLEARED`, `READY` or `WAITING`. A row in one of those
lanes on an early run is a finding worth reading, not a success.

### `graph-findings [--json] [--write] [path]`

`mev graph-findings` (`MV.ticket.graph-derived-carryover-findings`) closes a class of carryover
entries that were previously found only by an agent reading files: some findings are
**mechanically derivable from the corpus itself** — a lane file naming a block no `state.json`
registers, or a doc naming a script that exists nowhere in the fleet. This verb scans for those
deterministically, reusing the existing lane and graph readers rather than re-walking the
corpus, and can optionally write them straight into `carryover[]`.

```bash
mev graph-findings [--json] [--write] [path]
```

| Argument / Flag | Default | Description |
|---|---|---|
| `path` | `.` | Path to search from when locating `brain.toml` (walks up to find it) |
| `--json` | off | Emit the `GraphFindingsReport` as compact JSON instead of the human, detector-class-grouped summary |
| `--write` | off | Append each finding to its owning repo's `state.json` `carryover[]` as a typed entry (see **`--write`** below). Without this flag, `mev graph-findings` never modifies anything on disk — a `--json` run and a plain run are both read-only |

#### The two detectors

| Class | Meaning |
|---|---|
| `unregistered-lane-block` | An id in some `lane-*.json`'s `blocks[]` with no matching `tracks[].blocks[].id` in **that entry's own `repo`** field's `state.json` — a lane is not single-repo in this corpus, so ownership is resolved per-entry, never against the lane file's own location. Reuses `discover_lane_files` (`src/brain/lane_segments.rs`) and the existing state/block-graph readers (`src/brain/block_graph.rs`, `src/brain/state.rs`); a lane record that fails to parse is surfaced as an error diagnostic, never silently swallowed into a clean-looking zero |
| `referenced-path-absent` | A path named as a script or generator in a command or spec that resolves nowhere in the fleet, checked fleet-wide (see **Fleet-wide path resolution** below), not just against the referencing repo. Scanned sources are deliberately narrow: every `.md` under `<repo>/.claude/commands/` and every `.json` under `<repo>/planning/blocks/` — not READMEs, plans, decisions, or other prose, which would bury real findings under narrative mentions. A candidate reference is a `/`-containing path token ending in `.py` or `.sh` (the fleet's own generator/script extensions); bare filenames, URLs, and every other extension are excluded by design, not oversight. Resolution follows symlinks (`Path::exists` already does; the corpus walk sets `.follow_links(true)` explicitly), so a path that exists only through a `planning/` symlink into its `_planning/` vault is correctly reported present, never a false `absent` |

#### Fleet-wide path resolution (`referenced-path-absent`)

Measured on the live corpus 2026-08-23: checking a referenced path only against the repo that
references it made 25 of 31 distinct `referenced-path-absent` findings false (81%) — a shared
fleet script lives once, in the repo that owns the original, but the `.claude/commands/*.md` file
that references it is synced into every repo it was distributed to, so a repo-local-only check
reported the same real file "absent" in up to 19 places.

A referenced path is now resolved against these roots, **first match wins**:

| Order | Root | Base path |
|---|---|---|
| (a) | `repo:<repo>` | The referencing repo's own root — the only check that existed before this fix |
| (b) | `brain-root` | The fleet HQ root (`agentic-portfolio/`), for a path referenced relative to the brain rather than any one repo |
| (c) | `base-template` | `base-template/`'s own root — the source every `.claude/commands/*.md` file in the fleet is synced FROM, so a shared fleet script committed there resolves for every repo it was synced to |
| (d) | `owner:base-template` | Added only when the referencing file is a **synced command** — a `.claude/commands/*.md` whose basename also exists under `base-template/.claude/commands/`. Resolved through `BrainConfig`'s `base-template` `[[repos]]` entry, not a hardcoded path |

A path found under ANY of these roots is present — no finding is emitted. A path absent under
every root still produces exactly one finding per referencing repo (the fix narrows false
positives, it does not silence the detector), and that finding's `message` names every root that
was searched, by label and base path, so a future false positive is diagnosable from the
carryover entry alone without re-reading the source.

#### `finding_id` — why the same finding correlates across repos

Each finding carries a `finding_id`: a digest over **`(detector class, normalized subject)` and
nothing else** — never the owning repo, the file it was found in, a timestamp, or an index. This
is what makes the *same* finding, independently filed by several repos, correlate to one id: the
motivating case is `render-spec.py`, referenced (and missing) from `mev`, `base-template`, and
`engine-rs` alike, where `scripts/render_spec.py`, `./scripts/render_spec.py`, and
`base-template/scripts/render_spec.py` all normalize to the same subject and therefore the same
`finding_id`. `mev carryover`'s existing `finding_id` clustering groups these automatically once
written.

#### Exit codes

Unlike `mev carryover` (always exits `0` — it reports, it never gates), `graph-findings` is a
**gate-shaped reporter**:

| Code | Meaning |
|---|---|
| `0` | The corpus is clean — no findings and no error-severity diagnostic |
| `1` | At least one finding was reported, or an error-severity diagnostic was surfaced (e.g. an unparseable lane record), or `brain.toml` was not found/unreadable, or `--json` serialization failed |

#### `--write`

`--write` routes through `src/brain/carryover.rs` (`carryover_entry_for_finding` /
`write_graph_findings_for_repo`) rather than hand-serializing an entry in
`graph_findings.rs`, so every emitted entry inherits the existing `Carryover` shape and the
dispose sweep for free:

- **`scope`** is `{"repo": "<owning repo>", "tier": null, "cross_repo": null}` — exactly one
  non-null key, per the fleet-wide `scope` contract.
- **`kind`** is always `drift` — the honest fit for both detectors, since each is a fact held in
  two places that no longer agree (a lane's claim vs. `state.json`'s registry; a reference vs.
  the filesystem). `constraint` and `known_issue` are retired and are never minted here.
- **`finding_id`** is carried onto the entry verbatim, which is also the **idempotence key**: a
  finding whose `finding_id` already exists in that repo's `carryover[]` is skipped, so running
  `--write` twice never duplicates an entry and the verb is safe to run unattended.
- **`clears_when`** is a typed, brain-root-relative predicate (`MV.ticket.graph-findings-path-resolution`
  task 3), never `None` — every finding this verb emits is machine-detected from two data sources
  disagreeing, and that disagreement is itself machine-recheckable:
  - `referenced-path-absent` emits `FileExists { path: <raw reference>, .. }`. `path` is the raw,
    un-normalized reference (never the lossy normalized `subject`), spelled brain-root-relative so
    `mev carryover`'s own evaluator (`path_ref_satisfied` / `resolve_existing_path` in
    `src/brain/carryover.rs`) resolves it the same way this detector did for roots (a)/(b) —
    the evaluator only ever tries the brain root then the owning repo's `repo_path`, a narrower
    two-root check than the detector's four. A finding that resolved only via root (c)/(d) is
    never emitted in the first place, so the evaluator is never asked to reproduce a (c)/(d)-only
    verdict — only to notice a (a)/(b) repair.
  - `unregistered-lane-block` emits `FileContains { path: "<repo_path>/planning/state.json",
    pattern: "<block id>", .. }`, so the same evaluator's `file_contains_satisfied` resolves it.
  - Each predicate is guaranteed UNSATISFIED at the moment it is written — the detector just proved
    the path absent / the block unregistered — so `mev carryover`'s dispose sweep never retires a
    still-live finding on its first pass.
  - Entries written before this ticket carried no predicate at all (`clears_when: null`); the
    corpus reconciliation this ticket performed (`MV.ticket.graph-findings-path-resolution` task 6)
    removed the `referenced-path-absent` entries that now resolve and gave every survivor, plus
    every `unregistered-lane-block` entry, the typed predicate above.
- The rewrite is **byte-faithful** to the rest of the file: `state.json` is serialized with
  `ensure_ascii=False`-equivalent output, `indent=2`, and a trailing newline, so an append
  changes only the added array elements — the untouched portion of the file is byte-identical.
- Per-repo output names how many findings were newly appended (`"{repo}: appended N finding(s)
  to {path}"`), or `"{repo}: 0 appended (nothing new)"` when every finding for that repo was
  already present.

**Examples:**

```bash
# Human, detector-class-grouped summary of the whole fleet
mev graph-findings

# Machine-readable JSON envelope
mev graph-findings --json

# Report AND write new findings into each owning repo's carryover[]
mev graph-findings --write

# From an explicit brain root
mev graph-findings ~/Dev/agentic-portfolio
```

---

### `attention-queue [--out <path>] [--notify-only] [path]`

Emits every Attention-board item — across all four lanes (stale carryover's `Blocking`/`Hot`/
`Aging`/`Standing` sub-lanes, aging backlog, orphaned captures, and stale distilled knowledge) —
as a JSON array of `EN.8.A`-compatible operator payloads (`MV.ticket.attention-queue-delivery`).
This is how the Attention board stops being a surface somebody has to remember to open: instead of
running `/attention` and triaging the whole list in one sitting, `engine-rs`'s operator queue
(`EN.8.B`) can deliver one item at a time, in priority order.

`attention-queue` reuses the exact same corpus load and `effective_priorities` derivation that
`emit-state`'s attention-board planner (`plan_attention_board`) uses internally — the same
`collect_attention_rows` call, the same carryover union, the same backlog/distilled staleness
thresholds. There is only one board-derivation path in this codebase; the queue can never show a
different item, or a different order, than `/attention` itself would show.

#### Row-label precedence (carryover rows)

The markdown Attention board built by `emit-state` labels each carryover triage row
(`Blocking`/`Hot`/`Aging`/`Standing`, same four lanes as above) with the entry's authored
`summary` when one is present, rendered **verbatim** — never re-snippeted. Only when `summary` is
absent does the row fall back to the first 80 characters of `text` (`attention_snippet`, which
cuts mid-sentence and adds an ellipsis). This is a strict either/or: a present `summary` is never
clipped, truncated, or blended with `text`, no matter how long it is. A `summary` that is too long
to serve as a one-line label (multiline, or over 120 characters) is instead flagged at write time
by `W_STATE_CARRYOVER_SUMMARY_UNRENDERABLE` (see [`validate.md`](./validate.md)) — that is a
fixable authoring problem, not something the renderer silently papers over. `summary` is optional
by construction; an entry with none renders exactly as it always has.

| Argument / Flag | Default | Description |
|---|---|---|
| `path` | `.` | Path to search from when locating `brain.toml` (walks up to find it). |
| `--out <path>` | stdout | Write the JSON array to this file instead of printing it. |
| `--notify-only` | off | Cut the emitted set down to the interrupt subset only (see "Notification policy" below). Without this flag the output is the full ordered set, byte-identical to before this flag existed — depth limiting stays the queue's job, not this command's. |

#### Notification policy

`--notify-only` filters the already-ranked, already-ordered set down to the items allowed to
interrupt the operator, per the policy in `brain.toml`'s `[attention]` table. The rule itself —
why these four keys, and what the interrupt-vs-digest split is for — is decided in HQ's
`core/mev/docs/attention-triage-rule.md`; this section documents
only the mev-side surface, not the rule's rationale or its (dated, drifting) measurements.

| Key | Type | Default | Meaning |
|---|---|---|---|
| `notify_lanes` | `Vec<String>` | `["blocking", "hot"]` | `TriageLane` names eligible to interrupt at all. A lane not named here is excluded from `--notify-only` regardless of the other three keys — it still shows on `/attention` and still rolls into the digest. |
| `notify_priority_floor` | `u8` | `0` | Within the `hot` lane only, the highest (least urgent) priority number still eligible. Does not affect `blocking`. |
| `notify_blocking_any_priority` | `bool` | `true` | Whether `blocking` items interrupt regardless of priority, including items with no priority set. |
| `digest_everything_else` | `bool` | `true` | Whether the non-interrupt remainder is bundled into a once-daily digest rather than dropped; assembling that digest is `bastion:BA.21.D`'s side, not mev's. |

**The defaults fail closed.** An absent `[attention]` table, or a present one with none of these
four keys set, yields the documented rule — `blocking` at any priority plus `hot` at P0 only —
never "notify everything." A consumer (`bastion:BA.21.D`) should call `--notify-only` rather than
re-deriving this cut itself: a re-derived cut can diverge from what `/attention` and this command
agree on, which is the exact failure `attention-queue` exists to prevent.

#### Payload shape

Each array element is an `AttentionQueuePayload`:

| Field | Description |
|---|---|
| `item_id` | Stable identifier hashed from the item's IDENTITY only (repo, lane, slug) — never from mutable content. Re-running on an unchanged corpus reproduces it byte-for-byte; an item whose text/age/priority changed keeps its `item_id` and gets a new `digest` instead — a re-queue, not a new item, per `EN.8.A`. |
| `gate_id` | `"attention:<item_id>"` — excluded from the digest, per `OperatorPayload`'s own contract. |
| `rendered_summary` | Self-contained decision text: repo, lane, kind, slug, age, the item's text, and effective priority where present. An operator reading only the notification can decide without opening the repo. |
| `options` | 2–3 named `{key, label}` response options (see "Per-lane option sets" below). |
| `digest` | SHA-256 over `rendered_summary` + `options`, byte-identical to what `engine-core`'s `OperatorPayload::digest_of` computes for the same inputs — pinned by a hard-coded expected hex digest in this repo's test suite, so a future drift in either side's algorithm fails loudly instead of producing payloads the queue silently re-queues forever. |
| `effective_priority` | The post-propagation priority the board ranked this item at, supplied by mev rather than recomputed by the queue — `EN.8.B`'s `OperatorQueueItem::effective_priority` is enqueuer-supplied, never queue-computed. |
| `lane` | The carryover triage lane (`Blocking`/`Hot`/`Aging`/`Standing`), or absent for backlog/capture/distilled rows. |
| `repo` | The repo slug this item belongs to. |
| `source` | Provenance tag; `"attention-board"` for every item this command emits. |

The `gate_id`/`rendered_summary`/`options`/`digest` subset deserializes unchanged into
`engine-core`'s `OperatorPayload` — `item_id`, `effective_priority`, `lane`, `repo`, and `source`
ride alongside it as mev-owned fields the queue uses for ordering and provenance.

#### Per-lane option sets

`engine-core`'s `limits.rs` caps a payload at **3 response buttons**, **2 minimum**, and a
**20-character label limit** (confirmed against Meta's WhatsApp Cloud API docs 2026-08-12) — so
the board's five triage actions (promote · keep · snooze · resolve · archive) cannot ship as one
tap set. `attention-queue` resolves this by assigning each lane ≤3 options chosen for what that
lane can actually do, with every lane's set including a **session channel** option that routes the
operator to the full triage surface for any action that did not fit:

| Lane | Options | Why |
|---|---|---|
| Distilled (`knowledge.md`/`memory.md`) | Re-affirm, Open session | Never offers Snooze — HQ `CLAUDE.md`'s Attention rule: the distilled lane is re-affirmed by bumping `freshness:`, never snoozed. |
| Standing (carryover) | Keep, Open session | Never offers Promote or Resolve — `Standing` entries are permanently-true constraints, not items that graduate or close. |
| Blocking / Hot / Aging (carryover), Backlog, Capture | Promote, Snooze, Open session | The three actions that make sense for a genuinely time-bound item. |

A set outside 2..=3 options, or any label over 20 characters (measured in characters, not bytes —
labels may contain non-ASCII), is rejected in code at construction time rather than silently
truncated or dropped.

#### Ordering and stability

Items are sorted hottest-first by `effective_priority` (a lower number is hotter; an absent
priority sorts last), tie-broken by age descending then `item_id` ascending — a fully deterministic
order, so re-running on an unchanged corpus reproduces byte-identical output and a depth-1 queue
delivers the hottest item first.

#### Two boundaries this command does not cross

1. **mev derives; it does not project.** This command reads the corpus and emits an artifact —
   nothing more. It never enqueues into `engine-core`'s operator queue, opens a notification
   channel, or writes into `BA.18.A`'s sink. `engine-core`'s `queue/item.rs` module header states
   the crate does not read mev's state or shell out to `mev`; wiring this artifact into the queue
   (giving `ItemSource` a fourth, attention-sourced variant) is `engine-rs`'s change, made in a
   separate decision, not mev's.
2. **Depth limiting belongs to the queue, not mev.** `attention-queue` emits the *full* ordered
   set every run. `EN.8.B` holds items pending and releases one at a time — emission is not
   delivery. What mev owes is *correct ordering*; if `effective_priority` were wrong, depth-1
   delivery would faithfully deliver the wrong item first.

`attention-queue` is **read-only**: it never writes `state.json`, `BA.18.A`'s sink, or any
notification channel — only stdout, or the file named by `--out`. An empty board prints `[]` and
exits `0`; an empty queue is not an error.

**Examples:**

```bash
# JSON array to stdout, sibling brain repo at ..
mev attention-queue

# Explicit brain root
mev attention-queue ~/Dev/agentic-portfolio

# Write to a file instead of stdout
mev attention-queue --out /tmp/attention-queue.json

# Interrupt subset only, per brain.toml's [attention] notification policy
mev attention-queue --notify-only
```

---

