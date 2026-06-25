# Log Work — Sync status.md and append a Log entry for completed work.

## Variables

$ARGUMENTS — optional free-text explanation of what was done today. May be brief ("ported
             the parser spec and re-ran the test suite") or detailed (several
             sentences). If provided, it is woven into the Log prose entry as the primary
             narrative — do not discard or summarise it down. If omitted, derive the narrative
             from git history and the task spec alone.

## Execution Model

Spawn a Haiku subagent (Agent tool, `model: "haiku"`) to execute all steps below.
Pass the resolved `$ARGUMENTS` value and the complete Instructions section in the subagent prompt.
Return the subagent's result to the user.

## Instructions

1. Read `planning/status.md` and the current task spec at `planning/<name>/tasks.md`.
2. Run `git diff --stat` and `git log --oneline -10` to see what changed.

3. **Determine status.md changes — ask before writing if uncertain.**
   - From git history, the task spec, and `$ARGUMENTS`, identify which blocks are newly complete
     and what the next focus should be.
   - If you are confident (e.g. a block's acceptance criteria are clearly met and git confirms
     it), state the proposed changes and proceed.
   - If you are NOT certain which block(s) to flip to `Done`, or which block becomes the new
     current focus, STOP and ask the user:
     > "Before I update status.md, I want to confirm: should I mark [block X] as done and
     > set the current focus to [block Y]? Or did something differ from the plan?"
     Wait for confirmation before writing any status.md changes.
   - Changes to make once confirmed:
     - Flip newly-completed block statuses to `Done`.
     - Update the **Current focus** line to the next block.
     - Bump the **Last updated** date.
     - Append to the **Decisions & Deviations** log if reality diverged from the plan.

4. Append a new dated entry to `log.md` in this exact format:
   ```
   ## YYYY-MM-DD

   <One paragraph of prose: what was built or changed, why, and any notable decisions or
   surprises. If $ARGUMENTS was provided, use it as the primary narrative — include the
   user's own words and context, not just what git can infer.>

   ```diff
   <git diff --stat output, pasted verbatim>
   ```
   ```

5. **If a settled architectural choice was made during this work, record it as an atomic
   decision — ask first, never auto-author.**
   - Ask the user: "A settled choice came up — should I record it as a decision in
     `planning/decisions/`?" Wait for confirmation. Never write a decision unprompted.
   - On confirmation: read `planning/decisions/index.md` to find the last decision number;
     the new number is `last + 1` (D{N+1}).
   - Create `planning/decisions/D{N+1}-<kebab-title>.md` with OKF frontmatter:
     ```yaml
     ---
     type: Decision
     title: D{N+1} — Short Title
     description: One-sentence summary.
     ---
     ```
     followed by the body in the established form:
     ```
     ### D{N+1} — Short Title
     **Decided:** <what was decided>
     **Why:** <the reasoning>
     **Rejected:** <alternatives considered and why not> (optional)
     ```
   - Register it in `planning/decisions/index.md` by appending a row to the table (newest at
     the bottom — append-only; never edit or renumber prior entries).
6. Never edit the master plan file (`master-plan.md`).

7. **Sync the company brain.** After status.md and log.md are confirmed:
   - Read `../docs/projects/{{SLUG}}.md` in the company brain.
   - Update the **Current Status** date and focus line to match the new status.md state.
   - Update the Status column in this project's progress table for any rows that changed.
   - Open `../README.md` and find the Quick Status subsection for THIS project (the `###`
     heading matching this repo — the same project as the `docs/projects/*.md` you just
     read). Update its Current focus line and any changed status rows.
   - **Verify the section is THIS project's** before writing. If you cannot find a Quick
     Status subsection that clearly belongs to this project, STOP and report it — never edit
     another project's section, and never silently skip the sync.
   - Surgical updates only — do not rewrite sections that didn't change.
   - If the brain docs are genuinely already in sync, say so in your report (do not skip silently).

## Context / Files to Read

- `planning/status.md`
- The current `planning/<name>/tasks.md`
- `log.md`
- `../docs/projects/{{SLUG}}.md` (brain sync target)
- `../README.md` (brain sync target)
