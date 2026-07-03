# Worklog — ticket-ba15-12-okf-core-convergence

## Task 1 — PASSED (1 attempt)
What: Cargo.toml now depends on okf-core as an unpinned path dependency (../bastion/crates/okf-core, matching D15 discipline); cargo build --release succeeds with the new dependency present but unused.
Decisions: Literal Cargo.toml path text is ../bastion/crates/okf-core exactly as the ticket specifies (correct once this branch merges into the non-worktree core/mev/ checkout).; Because this worktree lives an extra 2 directories deeper (core/mev/trees/<name>/) than the eventual merge target (core/mev/), the literal relative path does not resolve from inside the worktree as-is. Created a local, untracked filesystem symlink core/mev/trees/bastion -> ../../bastion so cargo can resolve the path dependency for build validation now, without changing the committed Cargo.toml text. The symlink is not staged/committed.
Validated: gating checks (fast tripwire)
