---
type: Plan
title: "Ticket: guard emit-state --write against linked git worktrees"
description: mev emit-state --write resolves every repo's derived-file paths from brain.toml (root.join(repo_path)), never from CWD, so running it from inside a linked git worktree (e.g. base-template/trees/<slug>) silently regenerates the MAIN checkout's files instead of the worktree's own copy — refuse --write (or warn) when invoked from a linked worktree.
doc_id: update-write-state-in-trees
layer: [factory]
project: mev
status: active
keywords: [emit-state, worktree, brain.toml, state.json, race-condition, sdlc-flow, sdlc-block]
related: [MV.4.E, MV.4.D]
---

# Ticket: guard `emit-state --write` against linked git worktrees

## Metadata
prompt: `Fix mev emit-state --write so it refuses (or clearly warns) when invoked from inside a
linked git worktree, instead of silently regenerating the MAIN checkout's derived files. Root
cause found via live investigation in the agentic-portfolio brain — see Description.`
status: Not started
last-run: never

## Description

**Observed bug:** while running several `sdlc-flow`/`sdlc-block` orchestrations concurrently (each
in its own git worktree under `<repo>/trees/<slug>/`), uncommitted edits to a repo's
`planning/state.json` in the **main** checkout were being silently overwritten/reverted, with no
`git checkout`/`reset`/`clean` involved anywhere (confirmed by auditing the Rust TUI, the
orchestrator, and the JS workflow engines — none of them run destructive git commands on the main
tree). The real mechanism was traced to `mev emit-state --write`'s path resolution:

1. `EmitState { path, write }` (`src/main.rs:274-282`) calls
   `mev::brain::config::find_brain_root(&path)` (`src/brain/config.rs:138`), which walks **up**
   from `path` (default `.`, i.e. CWD) until it finds `brain.toml`. Walking up from
   `base-template/trees/<slug>/` lands on the exact same `agentic-portfolio/brain.toml` as walking
   up from `base-template/` itself — `trees/<slug>` is nested *inside* the repo, not a sibling
   directory, so this walk cannot tell the two apart.
2. `mev::emit_state(&root, write)` (`src/lib.rs:490`) then calls
   `discover_state_files(root, &config)` (`src/brain/state.rs:75`), which iterates every
   `[[repos]]` entry in `brain.toml` and resolves each one as `root.join(&repo.repo_path)` — the
   **fixed path registered in `brain.toml`** (e.g. `agentic-portfolio/base-template/planning/state.json`),
   never the CWD the command was actually invoked from.

Net effect: `mev emit-state --write` always reads/writes the same canonical **main-checkout**
files (`planning/state.json`, project-cache `README.md`/`status.md` rollups, tier rollups, the HQ
board, `master-plan.md` wave tables) for every repo in `brain.toml`, regardless of whether it was
invoked from that repo's main working tree or from a `trees/<slug>` linked worktree. `/log-work`
(`base-template/.claude/commands/log-work.md:86`, Step 3) and `sdlc-block.js`
(`base-template/.claude/workflows/sdlc-block.js:819` and `:1273`) both shell out to
`mev emit-state --write` as part of their normal flow. When several worktrees are doing this at
overlapping times, each one is independently regenerating and writing the *same* shared
main-repo files with no locking — a plain file-level race. If a human (or another agent) has an
uncommitted edit sitting in the main checkout of one of those files at that moment, a concurrent
worktree's background `emit-state --write` silently clobbers it. This is not a git-history revert
(no commit is touched) — it is an ordinary last-writer-wins overwrite of working-tree file
contents, which is why it looked like a mysterious "revert" with no reflog/stash trace.

**Fix direction (agreed with the user):** `emit-state --write` should refuse to run when CWD sits
inside a *linked* git worktree, rather than silently operating on the main checkout's files as if
nothing were different. A worktree's local state is not merged yet, so there is nothing correct
for `--write` to do from in there — the safe, correct place to run it is the main working tree
(which is what already happens at the end of a `/log-work` wrap-up or `sdlc-block.js`'s post-merge
refresh, when CWD really is the main root). Dry-run (no `--write`) is read-only and stays safe to
run from anywhere, so it is not gated.

## Relevant Files

- `src/main.rs` — the `Command::EmitState { path, write }` handler (~line 274-282); add the guard
  here, before `find_brain_root` is called, using the raw `path` argument (which is exactly the CWD
  the CLI was invoked from when the default `.` is used — this is the only place in the call chain
  that still has the *original*, unresolved invocation path; `find_brain_root`'s return value
  (`root`) has already lost that information by walking up to `brain.toml`). Also update the
  `EmitState` doc comment's "Diagnostic codes" list (~lines 113-118) to add the new failure code.
- `src/brain/config.rs` — add the new `is_linked_worktree(path: &Path) -> bool` helper next to
  `find_brain_root` (same file, same "resolve paths around `brain.toml`" concern). Runs
  `git -C <path> rev-parse --git-dir` and `git -C <path> rev-parse --git-common-dir` and compares
  them (canonicalized): a **linked** worktree's `.git` is a *file* pointing at
  `<main-repo>/.git/worktrees/<name>`, so `--git-dir` and `--git-common-dir` differ; in the **main**
  working tree they resolve to the same directory. Any failure to run `git` at all (not a repo, git
  not on `PATH`) must resolve to `false` (fail open) — see Notes on why this matters for existing
  tests.
- `core/mev/docs/cli.md` — the `emit-state` section (referenced from
  `base-template/.claude/commands/log-work.md:125`); document the new refusal behavior and its
  exit code so callers reading the CLI reference understand why `--write` can now fail from a
  worktree.

### New Files

None expected — the helper belongs next to `find_brain_root` in the existing `config.rs`, and the
guard belongs in the existing `EmitState` match arm in `main.rs`.

## Step by Step Tasks
See `tasks.json` in this directory — the task list is defined there, not here.

## Testing Strategy

- New unit tests for `is_linked_worktree` in `src/brain/config.rs`'s existing `#[cfg(test)] mod
  tests` block:
  - A real main-tree git repo (`git init` in a `tempfile::tempdir()`, one commit so `HEAD` exists)
    → `false`.
  - A real linked worktree of that repo (`git worktree add <path>`) → `true`.
  - A plain `tempfile::tempdir()` with **no** `git init` at all → `false` (must fail open — this is
    exactly the shape every existing `discover_state_files`/`emit_state` integration test in
    `tests/brain_emit.rs` already uses via `tempfile::tempdir()`, so failing open here is required
    to avoid regressing dozens of existing tests that were never git repos to begin with).
- New integration-style test (or extend `tests/brain_emit.rs`) exercising the CLI path: with
  `--write` inside a linked worktree, the process exits non-zero and prints a message naming the
  worktree path and pointing at running from the main tree instead; the SAME command with no
  `--write` (dry-run) from inside that same worktree still succeeds (exit 0) unchanged.
- Regression: `mev emit-state --write` invoked from the main working tree of a real repo (not a
  worktree) behaves exactly as before — this is the common case and must not regress.

## Acceptance Criteria

- `mev emit-state --write` run with CWD inside a linked git worktree exits non-zero, writes no
  files, and prints an error naming the worktree path and instructing the caller to run from the
  main working tree instead.
- `mev emit-state` (no `--write`, dry-run) run from inside a linked worktree still succeeds
  unchanged (reports its planned `W_EMIT_DRY_RUN` actions as before) — dry-run is never gated.
- `mev emit-state --write` run from a real repo's main working tree (not a worktree) is
  byte-for-byte unchanged in behavior from before this ticket.
- `is_linked_worktree` returns `false` (fail open) for any path that is not inside a git repository
  at all (e.g. a plain `tempfile::tempdir()`), so none of the existing `tests/brain_emit.rs`
  integration tests (which use bare tempdirs, no `git init`) regress.
- The new failure mode has a named diagnostic code (e.g. `E_EMIT_LINKED_WORKTREE`) documented
  alongside the existing `E_CONFIG_NOT_FOUND`/`E_EMIT_WRITE_FAILED` codes in `main.rs`'s `EmitState`
  doc comment and in `core/mev/docs/cli.md`.
- `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, `cargo build --release` all pass.

## Validation Commands

```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes

- **Scope boundary — mev only.** `bastion`'s `crates/bastion/src/brainval/mod.rs:183-186`
  (`run_emit_state`) is a second, independent pass-through that calls `mev::emit_state(&root,
  write)` directly as a library function, after resolving `root` itself via its own call to
  `mev::brain::config::find_brain_root`. Because this ticket's guard lives in **mev's CLI** (`main.rs`,
  gating on the raw CLI `path` arg before `find_brain_root` is called), it does **not** protect
  bastion's pass-through, which calls the library function directly and never goes through mev's
  `main.rs` at all. If bastion's own `bastion emit-state --write` can also be invoked from inside a
  worktree, that repo needs its own mirrored guard — out of scope here; flag it to the user as a
  bastion-side follow-up (mirroring how this project already mirrors cross-repo decisions, e.g.
  `D9-ba15-12-okf-core-convergence-mirror.md`).
- **Downstream behavior change to flag, not fix here.** `base-template/.claude/commands/log-work.md`
  Step 3 and `base-template/.claude/workflows/sdlc-block.js:819,1273` both shell out to
  `mev emit-state --write || true` — the `|| true` already swallows a non-zero exit, so those
  specific call sites will not break, they will just silently no-op (with a stderr message) on the
  rare occasion they happen to run from inside a worktree instead of the main root. Confirm (in
  base-template, not here) that no caller currently *depends on* `emit-state --write` succeeding
  from inside a worktree before this ships broadly — the two known call sites both document that
  they run from the MAIN repo root already, so this is expected to be a no-op change for them, but
  it's worth a quick downstream grep before wide rollout.
- Do not attempt to make `--write` "smart" about locating the worktree's *own* copy of
  `planning/state.json` and writing there instead — that would be a much larger change (per-repo
  path resolution would need to become CWD-aware everywhere, not just gated), and a worktree's
  local, unmerged state is not the right input for regenerating shared derived surfaces anyway.
  Refusing is the correct behavior, not redirecting.
- A related, but separate, bug was found in the same investigation: `base-template`'s
  `sdlc-block.js` orchestrator kept re-launching an already-merged block on `--resume` because its
  `load-state` agent call returns stale data instead of the freshly re-read state file. That bug
  lives in `base-template`, not `mev` — do not fold it into this ticket.

## Amendment Log
<!-- Append-only. Pipeline stages append one dated line here when they deviate from the plan. -->
_No amendments yet._
