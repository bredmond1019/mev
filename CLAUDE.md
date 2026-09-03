# CLAUDE.md — mev

@AGENTS.md

The file above carries everything that is true for any agent working in this repo: what `mev` is,
the orientation pointers, the standing rules, build/test/run, the directory map, the SDLC pipeline
notes, the response style and the stopping rule. **Read it as part of these instructions** — Claude
Code loads it automatically through the `@` import.

Only Claude-specific content belongs below.

## Fleet & Core Skills

The harness carries specialized skills in `.claude/skills/` (and `.agents/skills/`). Always consult
the corresponding skill before executing high-stakes fleet operations:

| Skill | Primary Focus | When to consult |
|---|---|---|
| **`commit-in-this-fleet`** | Safe git operations across multi-repo & vault symlinks | BEFORE any `git add`, `commit`, `stash`, `reset`, or `mv` |
| **`derive-state-safely`** | Authored vs derived state and writer execution | BEFORE running `mev emit-state --write`, `set-block-status`, or other state writers |
| **`edit-state-json`** | Canonical `planning/state.json` schema & graph edges | BEFORE hand-editing `state.json` or authoring `depends_on`/`carryover` |
| **`notify-operator`** | Operator alerting discipline via `bastion notify` | BEFORE sending notifications or deciding a lane is blocked |
| **`ping-agent`** | Cross-lane messaging envelopes & registry protocol | BEFORE sending or triaging cross-lane messages |
| **`report-to-the-operator`** | Concise operator reporting ceiling & format | When drafting chat replies, turn outputs, and run reports |
| **`run-the-gates`** | Fleet validation suite & gate diagnostics | BEFORE running `validate-brain` or `harness.json` checks |
| **`stop-or-continue`** | Session restart vs continuation correctness criteria | When an underlying binary/engine changes; never restart for token budget |
| **`write-okf-markdown`** | OKF YAML frontmatter & index.md row maintenance | BEFORE creating or editing any `.md` under `docs/` or `planning/` |
| **`write-repo-doc`** | Reader-first internal documentation standards | BEFORE writing or restructuring docs under `docs/` or guides |
