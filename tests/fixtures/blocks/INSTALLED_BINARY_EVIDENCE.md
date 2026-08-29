# Installed-binary evidence — `MV.ticket.block-record-validation`, Task 5

This records the un-gateable acceptance criterion: *"The installed mev binary the fleet runs
reports the new codes, not just the source tree the tests compile."* The in-repo `cargo
nextest`/`cargo test` suite compiles this working tree's source and structurally cannot observe
the installed artefact under `~/.cargo/bin/mev` — this file is that missing evidence.

**This checked installed behaviour, not source behaviour.**

## Procedure

```
cargo install --path . --force        # from the repo root; replaces ~/.cargo/bin/mev
mev --version                         # confirms the replaced binary is on PATH
mev validate-brain --state <corpus>   # <corpus> below
```

`<corpus>` is a disposable `brain.toml` corpus (not checked in) built by copying every directory
under `tests/fixtures/blocks/` into a scratch root and registering each as its own `[[repos]]`
entry (`repo_path` = the fixture's own directory name), with a minimal
`planning/state.json` per fixture declaring that fixture's own record id as an open block —
matching the "known ids" input `discover_block_records` + `check_block_record` expect, the same
way `tests/it/brain_block_records_fixtures.rs`'s `declared_id()` helper does in-process. The
`unknown-id` and `no-blocks-dir` fixtures are deliberately left without a `state.json` /
`planning/blocks/` respectively, per their own fixture contract.

## Result — installed `mev 0.1.0`, run against `tests/fixtures/blocks/`

```
$ mev validate-brain --state <corpus>
...
warning [W_BLOCK_MISSING_WHY] .../missing-why/planning/blocks/MV.fixture.missing-why.json — block record `MV.fixture.missing-why` has a missing or empty `why`
warning [W_BLOCK_MISSING_DESCRIPTION] .../missing-description/planning/blocks/MV.fixture.missing-description.json — block record `MV.fixture.missing-description` has a missing or empty `description`
warning [W_BLOCK_MISSING_OUT_OF_SCOPE] .../missing-out-of-scope/planning/blocks/MV.fixture.missing-out-of-scope.json — block record `MV.fixture.missing-out-of-scope` has a missing or empty `out_of_scope`
warning [W_BLOCK_SPEC_DIR_MISMATCH] .../spec-dir-mismatch/planning/blocks/MV.fixture.spec-dir-mismatch.json — block record `MV.fixture.spec-dir-mismatch` has spec_dir `planning/some-other-dir/`, expected `planning/MV.fixture.spec-dir-mismatch/`
warning [W_BLOCK_FILENAME_ID_MISMATCH] .../filename-id-mismatch/planning/blocks/MV.fixture.filename-id-mismatch.json — filename stem `MV.fixture.filename-id-mismatch` does not match record id `MV.fixture.wrong-internal-id`
warning [W_BLOCK_UNKNOWN_ID] .../unknown-id/planning/blocks/MV.fixture.unknown-id.json — block record `MV.fixture.unknown-id` has no matching block in state.json
warning [W_BLOCK_OPERATOR_EDGE_INCOMPLETE] .../operator-edge-incomplete/planning/blocks/MV.fixture.operator-edge-incomplete.json — block record `MV.fixture.operator-edge-incomplete` depends_on[0] is an operator edge missing `exit` or `start`
...
validated <corpus>: 0 error(s), 17 warning(s)
$ echo $?
0
```

All seven `W_BLOCK_*` codes named in the ticket's `what` fired exactly once, on the fixture that
targets each one. `known-good` and `no-blocks-dir` produced zero `W_BLOCK_*` diagnostics (the
other 10 warnings present in the full run are unrelated `W_STATE_FILE_MISSING` / pre-existing
`W_STATE_FOCUS_DRIFT` noise from the scratch corpus's minimal hand-written `state.json` files, not
from the block-record checks). Exit code is `0` — the run's 17 warnings, seven of them `W_BLOCK_*`,
do not fail it, confirming warning-only severity on the installed artefact.
