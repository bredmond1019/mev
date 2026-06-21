# Update Docs — Documentation health sweep: find stale sections and missing coverage.

Audits the entire documentation set against the current codebase and recent git history.
Produces a structured gap report (stale sections, missing coverage, confirmed-current).
Optionally applies surgical patches for clear-cut staleness with `--patch`.

This is the **ad-hoc maintenance** counterpart to `/document` (which gates on a PASS verdict
and is driven by a pipeline report). Use `/update-docs` for periodic doc health checks outside
the SDLC pipeline; use `/document` inside it.

## Variables

$ARGUMENTS — optional flags:
  - `--patch`        — after the audit, apply surgical fixes for clear-cut stale sections
  - `--since <ref>`  — limit git history to commits after this ref (default: last 20 commits)
  - A bare git ref/range (e.g. `HEAD~10`, `main..HEAD`) also sets the history window

## Instructions

### Phase 1 — Git history snapshot

1. Run `git log --oneline -20` (or `--since <ref>` if provided) to understand what changed recently.
2. Run `git diff HEAD~10 --stat` (or the specified range) to see which source files shifted.
3. Note any new commands, engine flags, schema fields, or decision records added since the last
   doc update. These are the primary staleness signals.

### Phase 2 — Codebase inventory

Sweep the source of truth for each documentation area. For each area, note: what exists now vs.
what the docs claim.

| Area | Sweep target | What to check |
|---|---|---|
| Commands | `.claude/commands/*.md` (list files + read first line of each) | Count, names, purpose lines |
| Workflow engines | `.claude/workflows/*.js` (scan for exported flags, agent names, schema fields) | New flags (`--from`, `--tasks`, etc.), new agents, changed behavior |
| Harness schema | `.claude/workflows/harness.schema.json` | Fields, types, new keys |
| Scaffold | `scaffold/planning/harness.json`, `scaffold/planning/harness.examples.md` | Profiles match docs |
| Decisions | `planning/decisions/` (list files, read titles) | New ADRs not reflected in docs |

### Phase 3 — Documentation inventory

Read every file under `docs/` plus `.claude/commands/README.md`. For each doc, record:
- What it claims to cover (commands listed, flags documented, fields described, decisions cited)
- The last section or table that mentions the most recently-changed area

Build a coverage matrix: `{ doc → [capabilities it covers] }` and the inverse
`{ capability → doc that covers it (or "undocumented") }`.

### Phase 4 — Gap analysis

For each discrepancy found, classify it:

**STALE** — doc references something that no longer matches source truth:
- A command count that's wrong (e.g., "22 commands" when there are 25)
- A flag or field listed in a doc that was removed, renamed, or changed behavior
- A decision number referenced that points to the wrong ADR
- An example or config snippet that no longer validates against the schema

**MISSING** — a capability exists in the codebase with no doc coverage at all:
- A new command added to `.claude/commands/` but not listed in README.md's table
- A new engine flag (e.g., `--from`, `--parallel-wave`) not documented anywhere
- A new `harness.json` field not in `docs/harness-json.md`
- A new workflow engine behavior not in `docs/architecture.md`

**NO-DOC** — code that was checked but deliberately should not get a doc:
- Internal implementation details (private functions, intermediate schema objects)
- ADR content — already captured in `planning/decisions/`, duplication would drift
- Scaffold template internals — those are the template, not user-facing docs

**CURRENT** — explicitly confirmed: doc section matches source truth.

### Phase 5 — Report

Output a structured report in this format:

```
## Documentation Health Report

### STALE — sections that need updating
- **<doc path>** § <section heading>
  What's wrong: <one line>
  Fix: <what the corrected text should say>

### MISSING — undocumented capabilities worth documenting
- **<capability>** (source: <file>)
  Suggested location: <which doc or "new doc: <name>">
  Why it warrants coverage: <one line>

### NO-DOC — checked, intentionally undocumented
- <thing> — <why no doc needed>

### CURRENT — confirmed up to date
- <doc path> — verified against <source files checked>
```

**Conservative threshold for MISSING:** only flag something as needing a new doc if:
- It is user-facing (a command, flag, config field, or workflow behavior)
- It is not already described in any existing doc, README, or inline comment visible from the
  docs/ inventory
- Adding a doc entry would reduce real confusion (not just increase coverage for its own sake)

### Phase 6 — Patch (only if `--patch` was passed)

For each **STALE** item where the fix is clear-cut (a count is wrong, a flag was renamed,
a field was added to a table):
- Apply the surgical edit described in the report.
- Leave the section marker `<!-- updated by /update-docs -->` if needed for auditability.
- Skip items marked as architecture-level changes — flag as `NEEDS_REVIEW` instead.
- Never touch `planning/` files, `log.md`, `status.md`, or `CLAUDE.md`.

After patching, list every doc edited with the specific sections changed.

## Rules

- **Audit first, patch only on request.** Without `--patch`, this command is read-only.
- **Source is authoritative.** If a doc and the source disagree, the source wins.
- **Conservative on MISSING.** Prefer fewer, high-value doc additions over comprehensive coverage
  of every internal detail. Three lines in an existing table beats a new standalone doc.
- **Surgical only when patching.** Never rewrite a doc section that wasn't identified as STALE.
- **Architecture-level changes → flag, don't edit.** Cross-cutting changes (engine dispatch
  logic, schema structure, OKF naming conventions) go to `NEEDS_REVIEW`, not auto-edited.
- **Do not touch** `planning/` files, `log.md`, `status.md`, or `CLAUDE.md`.

## Context / Files to Read

- `docs/` — all reference docs (architecture.md, using-the-template.md, harness-json.md, index.md)
- `.claude/commands/README.md` — the command reference
- `.claude/commands/*.md` — individual commands (list + first lines; full read only if flagged)
- `.claude/workflows/harness.schema.json` — schema source of truth
- `planning/decisions/` — ADR index (list files; read titles)
- `git log --oneline -20` and `git diff HEAD~10 --stat`
