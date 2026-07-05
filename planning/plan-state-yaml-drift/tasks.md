# Task Spec — Phase 5, Block A

**Status:** Not started · **Last run:** never

## Goal
Automate `now`/`next`/`blocked` scalar updates in `status.md` YAML frontmatter during `mev emit-state --write` to eradicate state drift.

## Context Pointers
- `planning/plan-state-yaml-drift/plan.md`
- `core/mev/src/brain/emit.rs` (where the planners live)
- `core/mev/src/lib.rs` (where `emit_state` is wired)

## Step-by-Step Tasks
See `tasks.json` in this directory — the task list is defined there, not here.

## Acceptance Criteria
- `mev emit-state --write` updates the `now`, `next`, and `blocked` scalars in the YAML frontmatter of `status.md` files to match the derived focus.
- Re-running `mev emit-state --write` on already synchronized files yields no changes (fixed-point property).
- Project's gating checks pass (see `planning/harness.json`).

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes

## Amendment Log
_No amendments yet._
