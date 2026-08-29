//! The single integration-test binary for `mev` — every file under `tests/it/` is a
//! module of THIS binary, not a binary of its own.
//!
//! Why: cargo builds one test binary per `tests/*.rs` file, and each one statically links
//! the whole crate plus its dependency graph. At 58 files that meant 58x the linking on
//! every full test run — measured 2026-08-27 as the dominant cost in this repo's build
//! (touching one line in `main.rs` took 53s to relink, dwarfing actual test runtime).
//! One binary, one link. Same pattern as engine-rs's `crates/engine-core/tests/it/`.
//!
//! Test ISOLATION is unaffected under `cargo nextest run` (this repo's mandated runner —
//! CLAUDE.md standing rule 6): it executes every test in its own process regardless of how
//! many binaries the tests are packed into. Plain `cargo test` runs them multi-threaded in
//! one process instead, which is why no test here may mutate global process state (e.g.
//! `env::set_current_dir`) — see the fix to `brain_config.rs`'s CWD test.
//!
//! Adding an integration test: create `tests/it/<name>.rs` and add one `mod <name>;` line
//! below. Do NOT add a new `tests/*.rs` file at this level — that silently reintroduces a
//! second binary.

mod approve_reject;
mod attention_queue;
mod blocks_driver;
mod blog_validate;
mod brain_block_graph;
mod brain_carryover;
mod brain_carryover_already_satisfied;
mod brain_carryover_archive_outflow;
mod brain_carryover_backfill;
mod brain_carryover_dedup;
mod brain_carryover_dispose;
mod brain_carryover_enforcement;
mod brain_carryover_grep_cli;
mod brain_carryover_ranking;
mod brain_carryover_trajectory;
mod brain_carryover_would_block;
mod brain_config;
mod brain_conformance;
mod brain_corpus;
mod brain_crawl;
mod brain_emit;
mod brain_epics;
mod brain_graph;
mod brain_graph_emit;
mod brain_last_touched;
mod brain_links;
mod brain_manifest;
mod brain_okf;
mod brain_quiesce_lease;
mod brain_state;
mod brain_structure;
mod brain_sync;
mod brain_validate;
mod build_stamp_cli;
mod check_consumers_cli;
mod close_operator_gate;
mod crawl;
mod doc_cli;
mod doc_index_reconcile;
mod doc_materialize;
mod doc_opportunity;
mod emit_block_graph_cli;
mod emit_state_lock;
mod emit_state_scope;
mod epic_lock;
mod fleet_regression;
mod force_operator_gate;
mod funnel_conformance;
mod graph_findings_cli;
mod lane_segments_dependency_split;
mod lane_segments_fleet;
mod lanes_driver;
mod master_plan_fixtures;
mod meta;
mod normalize_op_slugs;
mod reference_container;
mod set_block_status;
mod sibling_rules;
mod smoke;
mod state_history;
mod toolchain_freshness_write_banner;
mod validate_cli_flags;
mod validate_state_cli;
mod voice_tripwire;
