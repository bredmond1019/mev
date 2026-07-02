---
type: Handoff
created: 2026-07-02
---

# Handoff — state.json warnings resolved, naming convention adopted, /update-state shipped

> **For the next agent:** Read this immediately after `/prime`. Delete this file once consumed.

## What we're doing and why

The user ran `mev emit-state` / `mev validate-brain --state` against the live company brain and found
two classes of noise: (1) `W_EMIT_NO_SENTINEL` warnings on every repo's `master-plan.md`, and (2)
`E_STATE_SCHEMA_MISSING_FIELD` warnings on repos with empty `tracks[]`. Investigation split this into
real gaps, not mev bugs: some repos (`portfolio/` tier) are *terminal* — published to GitHub, never
going to have a roadmap — and were being treated as incomplete `project`-kind repos instead of a
distinct done state. Separately, the user asked to adopt a `<Prefix>.<Phase>.<Letter>` block-ID
convention (already used by mev itself, e.g. `MV.3B.U`) across `brazilianportugui` (BP), `amistad`
(AM), and `price-scout` (PS), and to wire that convention into `/generate-master-plan` +
`/new-project` so new projects get it automatically. Mid-session the user separately asked for a
command/skill so future agents know how to safely edit `state.json` — that shipped as `/update-state`.

## Completed this session

- **mev: `kind:"portfolio"` schema support** (`f5a0205`) — `discover_state_files` assigns
  `expected_kind:"portfolio"` when `brain.toml` `tier == "portfolio"`; `check_schema` accepts the new
  kind and requires a `note` instead of `tracks[]`; `plan_master_plan_tables` skips portfolio-kind
  files entirely (no `master-plan.md` expected). New unit/integration tests
  (`src/brain/state.rs`, `tests/brain_emit.rs`); `docs/cli.md` updated;
  `planning/decisions/D8-portfolio-kind-terminal-repos.md` written; `core/planning/state-schema.md`
  got the portfolio template section. All 4 harness gates green.
- Rewrote `portfolio/{rag-engine-rs,workflow-engine-rs,claude-sdk-rs}/planning/state.json` to
  `kind:"portfolio"` + a `note` — confirmed both warning classes gone for these three
  (`planning/` is gitignored there, so nothing to commit — the local files are sufficient for mev).
- **Naming convention rename**, `<Prefix>.<Phase>.<Letter>` (e.g. `BP.1.A`, task ids `BP.1.A.3`):
  - `amistad` (AM) — `master-plan.md` + `status.md` renamed, committed (`7eb9348` in that repo).
    `state.json` tracks[] was empty, nothing to remap there.
  - `price-scout` (PS) — same, committed (`bd19691` in that repo); historical log entries under
    `## Decisions & Deviations Log` were deliberately left in the old `Block X` phrasing (git history,
    not rewritten).
  - `brazilianportugui` (BP) — **NOT done**, see Remaining work below.
- **Command fixes** — `/generate-master-plan` (both the plain per-repo variant and the brain-flavored
  cross-repo variant) and `/new-project` updated so block headings read `### BA.0.A — <name>` (no
  redundant "Block" word) and `/new-project` now derives + registers a unique `prefix` in
  `brain.toml` instead of leaving it out. Distributed to all repos with a tracked `.claude/commands/`.
  - **Self-caught mistake, since fixed:** mid-session I overwrote the tier-level (root/core/side/
    client/portfolio) `.claude/commands/generate-master-plan.md` with the wrong (plain, not
    brain-flavored) variant. Caught via the `run_syncs.sh` topology check, reverted, and re-applied
    the heading fix to the correct `base-template/.claude/commands/brain/generate-master-plan.md`
    source (`9d9c135` root repo, `12868b5` base-template).
- **New `/update-state` command** — canonical workflow for editing any repo's `planning/state.json`
  (authored-vs-derived boundary, the `kind` decision table including the new `portfolio` kind, the
  block-ID rename checklist, edit → validate → `emit-state --write` → `validate-brain --state`
  procedure). Added in both plain and brain-flavored variants, registered in `run_syncs.sh`'s
  `sync-brain-commands` include list, distributed to all 12 repos' `.claude/commands/`.

## Remaining work

1. **`brazilianportugui` (BP) block-ID rename — still blocked, not started.** Re-checked
   2026-07-02: that repo now has a *different* live worktree (`1.F-contact-testimonial-...`) plus an
   actively running Claude Code process (PID observed, computer-use session) — still not safe to
   touch. Wait for that flow to settle (`git worktree list` there is clean, `git status` shows no
   surprise commits, and no live process), then reapply the same pattern used for `amistad`/
   `price-scout` (rename `Block X` headings in `master-plan.md`/`status.md` to `BP.<phase>.<letter>`,
   remap `state.json` `tracks[].blocks[].id` + `depends_on[].id` + `focus.next[].id`), and **commit
   immediately** once done. See the `brazilianportugui-block-id-rename-pending` carryover entry in
   this file's own `planning/state.json` (still open).
2. ~~`.agents/skills/generate-master-plan/` mirror drift~~ — **fixed 2026-07-02.** Regenerated
   `base-template/.agents/skills/generate-master-plan/SKILL.md` from the correct brain-flavored source
   (`base-template/.claude/commands/brain/generate-master-plan.md`, prefixed with the SKILL.md
   frontmatter) instead of the wrong plain per-repo command it had been copied from, and propagated the
   identical content to the root `.agents/skills/`, `core/`, `portfolio/`, `side/`, and `client/`
   mirrors (root's copy also predated the "no literal Block word" heading fix, so it needed the same
   update). Verified all six copies are byte-identical. `mev emit-state --write` +
   `mev validate-brain --state` re-run clean (0 errors, only pre-existing unrelated warnings). The
   `agents-skills-generate-master-plan-mirror-drift` carryover entry has been removed from
   `planning/state.json`.
3. Original mev roadmap is unaffected by any of this — `MV.3.L` (structural coverage, `index.md` ↔
   dir, D17) and `MV.3B.R` (graph emit / Phase 3B) are still the next actual mev feature blocks,
   per `planning/status.md`. Nothing this session touched mev's own feature roadmap.

## Durable State Updates

Two `carryover[]` entries added to `core/mev/planning/state.json` this session:
- `brazilianportugui-block-id-rename-pending` (`kind: deferred`, `scope: cross_repo`) — item 1 above.
- `agents-skills-generate-master-plan-mirror-drift` (`kind: known_issue`, `scope: cross_repo`) — item 2
  above.

No new `tracks[].blocks[]` entries — none of this session's work is a mev feature block; it's
cross-repo data/tooling work that happened to be done from inside mev.

## Open questions / choices

None — the approach for `brazilianportugui` is settled (same rename pattern as its siblings, just
deferred on timing); the `.agents/skills` drift fix, if picked up, should be scoped and decided fresh
by whoever tackles it (it may need its own investigation into whatever generates those mirrors).

## Context the next agent needs

Both open items are fully captured in `state.json` `carryover[]` (see Durable State Updates above) —
no additional session-only framing needed beyond what's there.

## First command after `/prime`

`git -C /Users/brandon/Dev/agentic-portfolio/client/brazilianportugui worktree list` — confirm the
concurrent SDLC flow has settled before resuming the `brazilianportugui` rename.
