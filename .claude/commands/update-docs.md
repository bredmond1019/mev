# Update Docs — Patch docs/*.md to reflect recent source changes (ad-hoc, git-diff driven).

Surgically updates existing reference docs when source code has changed, scoped by a git
diff range rather than a pipeline report. This is the **ad-hoc** counterpart to `/document`
(which gates on a review verdict and reads the implement report). Use `/update-docs` for
doc maintenance outside the SDLC pipeline; use `/document` inside it.

## Variables

$ARGUMENTS — optional git ref or range to diff against.

## Instructions

1. Determine the diff range:
   - If `$ARGUMENTS` is provided, use it as the git ref (e.g. `HEAD~3`, `main..HEAD`, a commit hash).
   - Otherwise default to `HEAD~1` (everything in the last commit) plus any unstaged changes.
   - Run `git diff --stat $ARGUMENTS` to get the list of changed source files.

2. Inventory which docs are affected:
   - Read every reference doc under `docs/` (recurse into subdirectories).
   - For each doc, check whether it explicitly references any of the changed source files
     (look for `**Source:**` annotations, code paths, function/component/class names that appear
     in the changed files).
   - Build a map: `{ doc_path → [changed_source_files_it_covers] }`.
   - If no docs reference any changed file, report "No docs affected by these changes" and stop.

3. For each affected doc + changed source pair:
   a. Read the full current doc.
   b. Read the full changed source file.
   c. Identify only the sections in the doc that describe the changed code (a function that
      was renamed, a prop that was added, a behavior that changed).
   d. Rewrite those sections to match the source exactly. Leave all other sections untouched.
   e. If a change is too large for surgical patching (e.g. an entire module/component was replaced),
      flag it as `NEEDS_REVIEW` and skip — do not attempt a full rewrite.

4. Report when done:
   - List every doc updated, with the specific sections changed.
   - List every doc checked but not needing updates.
   - List any `NEEDS_REVIEW` items with a one-line explanation of why.
   - If the scope of changes suggests a doc needs a wholesale refresh, say so and recommend a
     manual rewrite rather than silently producing a partial update.

## Rules

- **Surgical only.** Never rewrite a doc section that was not affected by the diff.
- **Source is authoritative.** If the doc and the source disagree, the source wins.
- **No invention.** Do not add new sections or cover new APIs/components not already in the doc.
- **Do not touch** `planning/` files, `log.md`, `status.md`, or `CLAUDE.md`. Those are managed
  by `/log-work`.
- **Architecture-level changes → flag, don't edit.** If the diff touches cross-cutting surfaces
  (shared libraries, build/config files, routing or middleware, or other architecture-level
  modules), flag the relevant doc as `NEEDS_REVIEW` rather than editing it automatically — a
  human confirms architecture-doc changes.
- **Respect the standing rules** (from CLAUDE.md): no fabricated metrics in docs.

## Context / Files to Read

- `docs/` — all existing reference docs
- Changed source files identified by `git diff --stat`
