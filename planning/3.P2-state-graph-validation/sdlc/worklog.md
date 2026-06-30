# Worklog — 3.P2-state-graph-validation

## Task 1 — PASSED (1 attempt)
What: Migrate src/brain/state.rs to the v2 state schema: add Origin/Backlog structs, extend TrackBlock with depends_on/wave/origin, rename Block.block and Endpoint.block to id with serde aliases for v1 compat, add backlog[] to StateFile, cascade all internal .block references to .id, and migrate fixture JSON keys to canonical v2 form.
Decisions: Used #[serde(alias = "block")] on Block.id and Endpoint.id rather than renaming the JSON key alone — this preserves backward compatibility with any v1 fixtures that still use the "block" key while making canonical v2 fixtures use "id".; Added backlog: vec![] to the two Rust StateFile struct constructors in test helpers so they compile against the new required field; all other StateFile instances are deserialized from JSON (which defaults via #[serde(default)]).; Migrated all fixture JSON strings to use "id" key (step 1.8) even though the serde alias makes this belt-and-suspenders — the spec explicitly requires canonical v2 form in the in-file fixtures.
Validated: gating checks (fast tripwire)

## Task 2 — PASSED (1 attempt)
What: Re-source graph DAG edges from tracks[].blocks[].depends_on[] (v2), add E_STATE_AUTHORED_BLOCKED and backlog status checks to check_schema, with 8 new unit tests
Decisions: focus.blocked_by[] is completely removed as an edge source — existing tests that used it for edge detection were updated to use tracks[].blocks[].depends_on[] instead; Integration tests in tests/brain_state.rs were also updated (task 6 owns them long-term but they needed immediate update to keep the suite green); Track blocks with invalid non-blocked status values are caught under E_STATE_SCHEMA_BAD_STATUS rather than a new code (consistent with existing focus status check)
Validated: gating checks (fast tripwire)

## Task 3 — PASSED (1 attempt)
What: Add detect_cycles (DFS, E_STATE_CYCLE with path) and ready_order (wave-ordered ready-open blocks, standalone for MV.3B.T) to src/brain/state.rs, with 13 unit tests covering all spec cases.
Decisions: detect_cycles_dfs is a private module-level function (not nested in detect_cycles) using fully-qualified HashMap/HashSet types to avoid import scope issues; ready_order accepts _graph parameter (unused, prefixed with underscore) for MV.3B.T forward-compat without triggering clippy unused-variable warning; wave=None treated as i64::MAX (lowest priority, goes last) for stable ordering; CrossRepo edges are excluded from cycle detection — only BlockedBy edges form the authoritative DAG
Validated: gating checks (fast tripwire)

## Task 4 — PASSED (1 attempt)
What: Added check_status_consistency (E_STATE_STATUS_INCONSISTENT) and check_backlog_integrity (E_STATE_DANGLING_BLOCKED_BY + E_STATE_DANGLING_PROMOTION) functions with 8 unit tests; all 267 state module tests pass and all harness gates green.
Decisions: check_status_consistency silently skips dangling deps (dep key not in status_map) to avoid double-reporting with check_state_graph's E_STATE_DANGLING_BLOCKED_BY; check_backlog_integrity emits E_STATE_DANGLING_PROMOTION for both cases: promoted node with no block pointer AND promoted node with a block pointer that resolves to nothing; Both functions are standalone public functions rather than extensions of check_state_graph, matching the spec's 'new check fns' language and keeping separation clean for task 6 wiring
Validated: gating checks (fast tripwire)
