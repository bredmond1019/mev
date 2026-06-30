# Worklog — 3B.T-state-table-rollup-emit

## Task 1 — PASSED (1 attempt)
What: Extracted derive_focus from check_focus_drift as single-source derivation; added DerivedFocus struct, derive_cross_repo, derive_rollup, and 8 integration tests covering all acceptance criteria for Task 1.
Decisions: DerivedFocus.blocked is Vec<(String, Vec<BlockedBy>)> returning only the unmet subset of depends_on (not the full list), matching the spec's requirement for emitter population of blocked_by[].; check_focus_drift now delegates entirely to derive_focus for the three-set derivation, then does set-comparison in place — no logic duplication.; derive_cross_repo skips same-repo and external deps; only cross-repo Block deps produce CrossRepoEdge.; derive_rollup filters children to kind==project before calling derive_focus, leaves tier as None per spec.; Used let-chain (if let ... && ...) to satisfy clippy::collapsible_if in derive_cross_repo.
Validated: gating checks (fast tripwire)
