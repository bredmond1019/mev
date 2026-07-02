# Log Work — Append a Log entry, sync status, and refresh the brain cache + tier rollup.

This command is **`brain.toml`-driven and depth-agnostic**. It works unchanged whether this repo lives
at the brain root, inside a tier sub-brain (`core/`, `portfolio/`, `side/`, `client/`), or standalone
outside any brain — it resolves everything from the manifest at runtime. There are **no baked `../`
paths and no `{{SLUG}}` token**; if you find either, it is a stale copy.

## Variables

$ARGUMENTS — optional free-text explanation of what was done. May be brief or detailed. If provided, it
is the primary narrative for the Log entry — do not discard or summarise it down. If omitted, derive the
narrative from git history and the task spec.

## Execution Model

Spawn a subagent (Agent tool) to execute all steps below; pass the resolved `$ARGUMENTS` and this whole
Instructions section in its prompt; return its result to the user. This command writes the **freshness
spine** (the watermarked cache + generated rollup that `mev validate-brain --sync` checks), so favour
accuracy over speed — use a capable model, not the cheapest.

## Instructions

### Step 0 — Resolve the manifest (this is what replaced the baked paths)

1. **Find the brain root.** From the current working directory, walk **up** parent by parent looking for a
   `brain.toml` file (its first line begins `# brain.toml`). The directory containing it is `BRAIN_ROOT`.
   - **If no `brain.toml` is found**, this is a **standalone repo**. Skip all brain-sync steps (3–4):
     do only Steps 1–2 (local `log.md` + `planning/status.md`) and say so in the report.
2. **Identify this repo in the manifest.** Compute this repo's path **relative to `BRAIN_ROOT`** (`.` if
   you are at the root). Read `brain.toml` and find the `[[repos]]` entry whose `repo_path` equals that
   relative path. From it read: `slug`, `tier`, `status_file`, `cache_doc`, `heading`.
   - If no entry matches (a repo not yet registered), STOP and report it — do not guess a cache path.
     (Registering a new repo is `/new-project` / `/generate-sub-brain`'s job.)
   - All `status_file` / `cache_doc` paths in the manifest are **relative to `BRAIN_ROOT`** — resolve
     them against `BRAIN_ROOT`, never against the current directory.

### Step 1 — Local `log.md`

3. Get today's date (`YYYY-MM-DD`) and the current ISO-8601 timestamp.
4. Open this repo's root `log.md` (`type: Log`; create it with OKF frontmatter if missing). Insert a dated
   entry — append a `### <title>` sub-entry under today's `## [YYYY-MM-DD]` section (create the date
   section at the top, just below `# Log`, if today's is absent):
   ```markdown
   ### <Short title of the session>
   - **What:** <what was built / changed / decided>
   - **Why:** <what prompted it — the problem, request, or insight>
   - **Refs:** <links to the driving plan / decision / tasks — else omit>
   ```
   **Why** is required; if `$ARGUMENTS` doesn't make the reason clear, ask one brief question first.
5. Update the `timestamp` field in `log.md`'s frontmatter to the current ISO-8601 time.

### Step 2 — Local `planning/status.md` (state + momentum)

6. Shell out to `mev emit-state --write` to automatically regenerate the **Current focus** line, the `now` / `next` / `blocked` scalars, and the tier rollup tables. Do NOT manually reimplement focus derivation.
   Then open this repo's `status_file` (`type: ProjectStatus`) and update **surgically**:
   - the `timestamp` frontmatter field → current ISO-8601 time;
   - bump the `## Metrics` counters if present (cheap; skip if the file has none).
   - If you are **not certain** what the new focus is, state your
     proposed change and ask before writing.
7. **Settled decision?** If a settled architectural choice was made, ask whether to record it in
   `planning/decisions/` (never auto-author). On yes: next number from `planning/decisions/index.md`,
   create `D{N+1}-<kebab-title>.md` with OKF frontmatter, append a row to the decisions index.
8. **Never edit `master-plan.md`** from this command.

> If `BRAIN_ROOT` was not found (standalone repo), stop here and report.

### Step 3 — Brain cache (the one watermarked source per repo)

9. **Skip this step when `slug == "brain"`** (the brain root's `cache_doc` is its own `README.md` — handle
   that in Step 3b). Otherwise update the cache at `BRAIN_ROOT/<cache_doc>`:
   - Update its **Current Status** date + focus line and any changed rows to match the new
     `status.md` state (surgical edits only).
   - Set its frontmatter **`synced_from`** to the **source `status.md`'s `timestamp`** value (full
     ISO-8601). This watermark is what `mev validate-brain --sync` compares — `source.timestamp ==
     cache.synced_from` must hold, so copy it verbatim.
   - If the cache is genuinely already in sync, say so (don't skip silently).

   **Step 3b — `_root` repos.** For repos whose `tier == "_root"` (the brain itself, `learn-ai`,
   `base-template`), there is **no tier rollup**. The brain root additionally keeps a hand-maintained
   `## Quick Status` in its `README.md`; update only THIS repo's `###` subsection there (verify the
   heading matches `heading` before writing — never touch another project's subsection). Then go to
   Step 5.

### Step 4 — Regenerate the tier rollup (tiered repos only)

10. If `tier` is one of `core` / `portfolio` / `side` / `client`, regenerate that tier's rollup table:
    - Open `BRAIN_ROOT/<tier>/planning/status.md`.
    - Replace **only** the lines between `<!-- ROLLUP:BEGIN ... -->` and `<!-- ROLLUP:END -->`. **Do not
      touch** the `## Momentum`, `## Metrics`, or any other section — those are hand-maintained.
    - Rebuild the table with one row per `[[repos]]` entry sharing this `tier` (in manifest order). Each
      row: the project name linked to its cache (`../docs/projects/<slug>.md`), its one-line current
      focus (from that cache's Current Status line), and its `synced_from` date (or `—` if unset). This
      is light reading of each cache's focus line + watermark — no deep parsing.
    - If the markers are missing (older tier file), insert them under a `## Rollup — repos in this tier`
      heading and report that you added them.

### Step 5 — Report

11. Show the user: the `log.md` entry added; the `status.md` fields changed; the cache file written +
    its new `synced_from`; the tier rollup regenerated (or "_root — no rollup"); and any decision
    recorded. If standalone, say the brain sync was skipped.

## Context / Files to Read

- `brain.toml` (at `BRAIN_ROOT` — the manifest; resolve everything from it)
- this repo's `log.md` and `status_file` (`planning/status.md`)
- the current `planning/<concept>/tasks.md`, if any
- `BRAIN_ROOT/<cache_doc>` (brain cache target — except `slug == brain`)
- `BRAIN_ROOT/<tier>/planning/status.md` (tier rollup target — tiered repos only)
