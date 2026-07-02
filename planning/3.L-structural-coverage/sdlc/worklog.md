# Worklog — 3.L-structural-coverage

## Task 1 — PASSED (1 attempt)
What: Added src/brain/structure.rs implementing check_structure(corpus, root) — bidirectional index.md/directory coverage, emitting E_STRUCT_ORPHAN_FILE and E_STRUCT_DANGLING_ROW diagnostics, registered via pub mod structure in src/brain/mod.rs, with 7 unit tests.
Decisions: Identified index.md entries by checking entry.path.file_name() == Some("index.md") rather than relying on stem alone, to be precise and match existing crawl conventions.; Reused links::extract_links + a local lexical normalize() helper (mirroring links.rs::normalize_path) rather than adding a shared cross-module helper, since the task spec called for a small private helper local to structure.rs.; FileUri targets are resolved as absolute paths (stripping the file:// scheme) rather than joined to the index.md's directory, consistent with check_links' existing FileUri handling.
Validated: gating checks (fast tripwire)

## Task 2 — PASSED (1 attempt)
What: Added validate_brain_structure library driver (schema pass + crawl + check_structure) and wired a --structure CLI flag into ValidateBrain, dispatched ahead of --state/--graph/--sync but after --links.
Decisions: Dispatch precedence chosen as: --links > --structure > --state > --graph > --sync > default, documented in the --structure flag's doc comment on the ValidateBrain subcommand.; Left the pre-existing '(in progress)' marker on tasks.md ### 3.L.2 heading untouched/unstaged since it predates this task and updating task-spec prose is outside this task's file ownership (src/lib.rs, src/main.rs only).
Validated: gating checks (fast tripwire)

## Task 3 — PASSED (1 attempt)
What: Added end-to-end integration tests (tests/brain_structure.rs) exercising validate-brain --structure: clean tree (0 diagnostics, exit 0), orphan file detection (E_STRUCT_ORPHAN_FILE), dangling row detection (E_STRUCT_DANGLING_ROW), JSON envelope carrying both codes, and a CLI subprocess end-to-end check for exit codes.
Decisions: Followed the fixture-construction style of tests/brain_links.rs (write_file/temp_dir/write_brain_toml helpers) for consistency across brain_*.rs integration test files.; Combined dirty/clean CLI exit-code assertions into one test (cli_structure_flag_end_to_end) rather than two, mirroring the existing links_flag_outranks_state_in_dispatch precedent of testing CLI dispatch via CARGO_BIN_EXE_mev subprocess.
Validated: gating checks (fast tripwire)

## Task 4 — PASSED (1 attempt)
What: Documented the --structure validate-brain flag (dispatch precedence, E_STRUCT_ORPHAN_FILE/E_STRUCT_DANGLING_ROW codes, examples) in docs/cli.md, and added the structure.rs module + Structural coverage section to docs/architecture.md, mirroring the existing --links / links.rs documentation shape.
Decisions: Placed the new --structure section in docs/cli.md between --state and --links (matching its dispatch precedence position: below --links, above --state/--graph/--sync), and updated the precedence prose in both the --state section note and the flag table to mention --structure explicitly.; In docs/architecture.md added a full 'Structural coverage (src/brain/structure.rs) — Phase 3 Block L' section (public function table, diagnostic locator table, entry-point description) mirroring the Link integrity section's shape, plus updated the module map and tests/ listing (brain_structure.rs).
Validated: gating checks (fast tripwire)

## Task 5 — PASSED (1 attempt)
What: Validated MV.3.L structural-coverage work: all four harness gates green (fmt, clippy -D warnings, cargo test incl. new structure.rs unit tests + brain_structure.rs integration tests, release build) and mev validate-brain --structure sanity-checked against the live company brain (84 genuine E_STRUCT_ORPHAN_FILE findings, 0 dangling rows, no false positives).
Decisions: Fixing the brain's own orphaned index.md rows found by the live sanity check is out of scope for MV.3.L (a validator-behavior spec, not a brain-content cleanup task) — noted in tasks.md Notes rather than acted on.; Only edited Task 5's own checklist line (marked done) plus the Notes section, per instructions to touch only Task 5 scope; left Task 4's '(in progress)' marker untouched since it belongs to a different task.
Validated: gating checks (fast tripwire)
