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
mod brain_block_create;
mod brain_block_graph;
mod brain_block_records_fixtures;
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
// Learn-ai's file crawler (`mev::crawl`) is feature-gated (see src/lib.rs); this suite
// exercises it directly and so only compiles/runs under the `learn-ai` feature. Left in
// mev's own tests/it (not moved to crates/mev-learn-ai) since it predates and is outside
// this block's declared scope (funnel/voice/blog/validate_cli_flags only) — feature-gating
// here is the minimal fix for a default `cargo test` to compile at all.
#[cfg(feature = "learn-ai")]
mod crawl;
mod doc_cli;
mod doc_index_reconcile;
mod doc_materialize;
mod doc_opportunity;
mod emit_block_graph_cli;
mod emit_state_authored_roundtrip;
mod emit_state_lock;
mod emit_state_scope;
mod epic_lock;
mod fleet_regression;
mod force_operator_gate;
mod graph_findings_cli;
mod lane_segments_dependency_split;
mod lane_segments_fleet;
mod lanes_driver;
mod master_plan_fixtures;
// Learn-ai's struct/frontmatter validator (`mev::validate_file`) is feature-gated; see the
// `crawl` mod comment above for why this stays here, gated, rather than moving.
#[cfg(feature = "learn-ai")]
mod meta;
mod normalize_op_slugs;
mod reference_container;
mod set_block_status;
mod sibling_rules;
// Exercises `mev::validate()`, which is feature-gated; see the `crawl` mod comment above.
#[cfg(feature = "learn-ai")]
mod smoke;
mod state_history;
mod toolchain_freshness_write_banner;
// Tests `mev`'s own `validate` subcommand CLI wiring (`--blog`/`--lint` flags, JSON envelope
// label, exit codes) by driving the built binary — that subcommand exists only behind the
// `learn-ai` feature (see src/main.rs), so this suite only compiles/runs under it. It stays a
// module of mev's own tests/it binary rather than moving into crates/mev-learn-ai: it exercises
// the mev *binary* (`CARGO_BIN_EXE_mev`), which is only available to a package's own integration
// tests, not to a dependent crate's.
#[cfg(feature = "learn-ai")]
mod validate_cli_flags;
mod validate_state_cli;
