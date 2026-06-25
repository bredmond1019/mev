# /patch — Lightweight hotfix pipeline

> **Warning:** This workflow skips test, review, and document stages. Use only for low-risk
> hotfixes. If you are unsure whether a change is patch-scope, use `/sdlc-run` instead.

## What this is

A stripped-down pipeline for hotfixes and docs-only changes: implement → validate → commit.
No test report, no review stage, no document stage. Appropriate when:
- The change touches one file (or a tightly coupled pair).
- No new tests are needed (no new logic introduced).
- No API surface, schema, or interface changes.
- The fix is self-evidently correct and low blast-radius.

## Instructions

### Step 1 — Scope check

Before doing anything, confirm the change is patch-scope. Answer each question:

1. Does the change touch more than two files? → If yes, stop and use `/sdlc-run`.
2. Does the change introduce new logic that requires a test? → If yes, stop and use `/sdlc-run`.
3. Does the change affect an API endpoint, schema, or public interface? → If yes, stop and use `/sdlc-run`.

If you cannot answer all three with "no", output:

```
SCOPE CHECK FAILED — this change is not patch-scope.
Redirect: use /sdlc-run for full pipeline coverage.
```

And stop.

### Step 2 — Implement

Apply the fix. Read the affected file(s), make the targeted change, verify the edit looks correct.

Do not reorganize, refactor, or touch anything outside the minimal fix.

### Step 3 — Validate

Read `planning/harness.json`. Run every command listed under `validation.checks[]` whose
`kind` is `"command"` (the fast checks). Do not run the full test suite if it takes more than
30 seconds — the goal is a quick sanity check, not a full regression run.

If any check fails, fix it before proceeding. If the check failure is unrelated to the patch
and pre-existing, note it but do not block the commit.

### Step 4 — Commit

Run `/commit` with a `fix:` prefix describing the hotfix in one line.

### Step 5 — Done

Output a one-line summary:
```
Patch complete: <what was fixed> — <file(s) changed>
```

Remind the user: "Patch workflow skips test/review/document. Run /sdlc-run if you want full
pipeline coverage on this change."
