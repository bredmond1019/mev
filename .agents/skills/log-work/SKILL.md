---
name: log-work
description: Log a completed work session — append a dated log.md entry, sync planning/status.md (state + momentum), refresh the brain.toml-resolved cache with a synced_from watermark, and regenerate the owning tier's rollup. Depth-agnostic; reads brain.toml.
---

# Log Work — Append a Log entry, sync status, and refresh the brain cache + tier rollup.

This skill is **`brain.toml`-driven and depth-agnostic** — it works unchanged whether this repo lives at
the brain root, inside a tier sub-brain (`core/` · `portfolio/` · `side/` · `client/`), or standalone
outside any brain. No baked `../` paths, no `{{SLUG}}` token.

## Parameters

- **Work Summary** (optional): description of the work completed. If provided, it is the primary narrative
  for the log entry; if omitted, derive it from git history and the task spec.

## Instructions

### Step 0 — Resolve the manifest
1. **Find the brain root:** walk up from the cwd for `brain.toml` (first line begins `# brain.toml`); its
   directory is `BRAIN_ROOT`. **If none is found, this is standalone** — do only Steps 1–2, skip 3–4.
2. **Identify this repo:** compute the cwd's path relative to `BRAIN_ROOT` (`.` at root); find the
   `[[repos]]` entry whose `repo_path` matches. Read `slug`, `tier`, `status_file`, `cache_doc`,
   `heading`. All manifest paths are relative to `BRAIN_ROOT`. If no entry matches, STOP and report.

### Step 1 — Local log.md
3. Get today's date + ISO-8601 timestamp.
4. Open this repo's `log.md` (`type: Log`; create with OKF frontmatter if missing). Append a
   `### <title>` sub-entry under today's `## [YYYY-MM-DD]` (create the date section at top if absent),
   capturing **What / Why / Refs**. **Why is required** — ask one brief question if unclear.
5. Update the `log.md` frontmatter `timestamp`.

### Step 2 — Local planning/status.md
6. Open `status_file` (`type: ProjectStatus`). Surgically update: `timestamp`; **Current focus** + changed
   block statuses; the `## Momentum` `now`/`next`/`blocked` queues; the matching **frontmatter scalars**
   (D30 — keep them mirrored); bump `## Metrics` if present. If unsure which block is done / what the new
   focus is, propose and ask before writing.
7. **Settled decision?** Ask before recording one in `planning/decisions/` (never auto-author); on yes,
   number from the decisions index, create `D{N+1}-<kebab>.md`, append the index row.
8. Never edit `master-plan.md`. *(Standalone: stop and report here.)*

### Step 3 — Brain cache (one watermarked source per repo)
9. **Skip if `slug == brain`** (its cache is its own README — see 3b). Else update `BRAIN_ROOT/<cache_doc>`:
   refresh the Current Status date + focus + changed rows (surgical); set frontmatter **`synced_from`** to
   the **source status.md `timestamp`** value verbatim (full ISO — this is what `mev validate-brain
   --sync` compares).
   - **3b — `_root` repos** (brain, learn-ai, base-template): no tier rollup. For the brain root, update
     only THIS repo's `###` subsection in `README.md`'s `## Quick Status` (verify the heading first).
     Then go to Step 5.

### Step 4 — Tier rollup (tiered repos only)
10. If `tier` ∈ {core, portfolio, side, client}: in `BRAIN_ROOT/<tier>/planning/status.md`, replace **only**
    the `<!-- ROLLUP:BEGIN --> … <!-- ROLLUP:END -->` block — never touch `## Momentum` / `## Metrics`.
    One row per `[[repos]]` entry in this tier (manifest order): linked project · one-line current focus ·
    `synced_from` date (`—` if unset). Insert the markers if missing and report it.

### Step 5 — Report
11. Report: log entry added; status fields changed; cache written + new `synced_from`; rollup regenerated
    (or "_root — no rollup"); decision recorded (if any). If standalone, say brain sync was skipped.
