# Task Spec — Phase 2, Block I

**Status:** Not started · **Last run:** never

## Goal
Wire `BrainValidator` to a `mev validate-brain <brain-root>` subcommand and add a global `--json` flag emitting a machine-readable envelope the Brain RAG indexer can consume as a pre-`--rebuild` gate.

## Context Pointers
- **Plan section:** `planning/master-plan.md` → Phase 2 → *Block I — `validate-brain` subcommand + JSON reporter*. Its "What" and acceptance bar are authoritative.
- **Current CLI surface:** `src/main.rs` — a `clap` `Cli` with a single `Validate { path }` subcommand, a stub human reporter (`println!("validated … N error(s) …")`), and exit-code mapping via `report.is_failure()`. The `about` text still reads "Validate and compile learn-agentic-ai.com content" — Block I broadens it to name both consumers.
- **Library surface to build on:** `src/lib.rs` already re-exports `BrainValidator` and defines `Diagnostic`, `Severity`, `Report` (+ `error_count`/`warning_count`/`is_failure`), and the learn-ai `pub fn validate(root) -> anyhow::Result<Report>` wrapper. `BrainValidator.run(root)` (in `src/brain/mod.rs`) is the entry point: it applies Block G's crawl skip-list (nested-git + `target/`) and Block H's OKF checks. Block I adds a parallel `validate_brain` wrapper and the JSON serialization.
- **Deps available:** `serde` (with `derive`), `serde_json`, `clap` (with `derive`), `anyhow` — all already in `Cargo.toml`. No new dependency is needed.
- **Standing rules:** every block ships with tests (CLAUDE.md rule 1); all four harness gates stay green.
- **Scope boundary (from the plan):** the `validate-brain` subcommand, the `--json` flag + envelope, `Serialize` on the diagnostic types, and the `about`-text update. Do **not** add the learn-ai `--json` path beyond what falls out of the shared flag, and do **not** change the learn-ai validation logic.

## Step-by-Step Tasks

### 1. Serializable diagnostics + `validate_brain` wrapper + JSON envelope
- In `src/lib.rs`: derive `serde::Serialize` on `Severity` and `Diagnostic`. Serialize `Severity` as lowercase (`#[serde(rename_all = "lowercase")]`) so the envelope reads `"error"`/`"warning"`. Keep `PathBuf` fields serializing as their string path (serde's default).
- Add `pub fn validate_brain(root: &std::path::Path) -> anyhow::Result<Report>` mirroring `validate()` — delegates to `BrainValidator.run(root)`.
- Define a `#[derive(Serialize)]` envelope struct (e.g. `pub struct JsonReport`) with the fields the plan names: `validator: &str` (e.g. `"brain"`), `root: String` (the root path display), `errors: usize`, `warnings: usize`, `diagnostics: Vec<Diagnostic>` (borrow or clone from the `Report`). Add a `pub fn` that builds it from a `(validator, root, &Report)` and a helper that serializes it to a JSON string via `serde_json::to_string_pretty`.
- Files: `src/lib.rs` (modified).

### 2. `validate-brain` subcommand + global `--json` flag in the CLI
- Depends on Task 1. In `src/main.rs`: add a `ValidateBrain { path }` variant to the `Command` enum with `#[arg(default_value = "..")]` (the brain root defaults to the parent dir, per the plan). Add a global `--json` flag (e.g. `#[arg(long, global = true)]` on `Cli`) so it applies to either subcommand.
- Dispatch: `Command::ValidateBrain { path }` calls `mev::validate_brain(&path)`. When `--json` is set, print the serialized JSON envelope (Task 1) to stdout; otherwise print the existing human one-line summary. Preserve exit-code mapping (`is_failure()` → `ExitCode::FAILURE`).
- Wire `--json` into the existing `Validate` arm too (it is a global flag): emit a JSON envelope with `validator: "learn-ai"` when set, else the current human summary. Keep both arms’ non-JSON output unchanged.
- Update the `#[command(about = …)]` text to describe both consumers (learn-ai content + Bastion Brain OKF).
- Files: `src/main.rs` (modified).

### 3. Tests for the brain wrapper + JSON envelope
- Depends on Tasks 1–2. Add `tests/brain_validate.rs` (new integration test file) exercising the public library surface:
  - Build a temp "brain-like" fixture dir (same temp-dir style as `tests/meta.rs`/`tests/brain_crawl.rs`): a root-level `.md` with a deliberate OKF violation (e.g. missing `title`), plus a nested sub-dir containing a `.git` marker and a `.md` inside it — assert `validate_brain` flags the root file and **does not** descend into the nested-git sub-dir (proves the crawl skip-list is honored end-to-end).
  - Assert the JSON envelope from Task 1 serializes to valid JSON, round-trips via `serde_json::from_str` into a `serde_json::Value`, and contains the keys `validator`, `root`, `errors`, `warnings`, `diagnostics`, with `errors`/`warnings` matching the `Report`'s counts and `diagnostics` length matching the diagnostic count.
- Files: `tests/brain_validate.rs` (new).

### 4. Validate
- Run the Validation Commands listed below and confirm all pass.

## Acceptance Criteria
- `mev validate-brain <root>` exists, defaults `<root>` to `..`, runs `BrainValidator`, and reports real OKF violations across the brain corpus while skipping every nested-git sub-project and `target/`.
- A global `--json` flag is accepted by both `validate` and `validate-brain`; with it set, the command prints a valid JSON envelope with keys `validator`, `root`, `errors`, `warnings`, `diagnostics[]`; without it, the existing human summary is printed unchanged.
- `Severity` and `Diagnostic` implement `serde::Serialize`; `Severity` serializes as lowercase `"error"`/`"warning"`.
- `pub fn validate_brain(root) -> anyhow::Result<Report>` is exposed from the library, mirroring `validate()`.
- Exit code is `FAILURE` when any error-severity diagnostic is present, `SUCCESS` otherwise, in both human and `--json` modes.
- The CLI `about` text names both consumers (learn-ai content + Bastion Brain OKF).
- New integration tests prove `validate_brain` honors the crawl skip-list end-to-end and that the JSON envelope is valid and carries the expected keys/counts.
- The existing learn-ai, Block G, and Block H tests are unchanged and still pass; all four harness gates pass.

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```
<!-- Manual acceptance (not gated): `cargo run -- validate-brain ~/Dev/agentic-portfolio --json` emits valid JSON and skips sub-projects. -->

## Notes
<filled in as work happens>

## Amendment Log
<!-- Append-only. Pipeline stages append one dated line here when they deviate from the spec. -->
_No amendments yet._
