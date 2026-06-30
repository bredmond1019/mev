# Worklog — 3.P2-state-graph-validation

## Task 1 — PASSED (1 attempt)
What: Migrate src/brain/state.rs to the v2 state schema: add Origin/Backlog structs, extend TrackBlock with depends_on/wave/origin, rename Block.block and Endpoint.block to id with serde aliases for v1 compat, add backlog[] to StateFile, cascade all internal .block references to .id, and migrate fixture JSON keys to canonical v2 form.
Decisions: Used #[serde(alias = "block")] on Block.id and Endpoint.id rather than renaming the JSON key alone — this preserves backward compatibility with any v1 fixtures that still use the "block" key while making canonical v2 fixtures use "id".; Added backlog: vec![] to the two Rust StateFile struct constructors in test helpers so they compile against the new required field; all other StateFile instances are deserialized from JSON (which defaults via #[serde(default)]).; Migrated all fixture JSON strings to use "id" key (step 1.8) even though the serde alias makes this belt-and-suspenders — the spec explicitly requires canonical v2 form in the in-file fixtures.
Validated: gating checks (fast tripwire)
