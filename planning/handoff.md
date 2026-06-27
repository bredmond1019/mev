---
type: Handoff
created: 2026-06-26
---

# Handoff — Block 2.H done; start Block 2.I (validate-brain subcommand + --json)

> **For the next agent:** Read this immediately after `/prime`. Delete this file once consumed.

## What we're doing and why

`mev` is being generalised into a two-consumer tool: learn-ai content validation (Phase 1)
and Bastion Brain OKF frontmatter validation (Phase 2). The motivating goal is
`mev validate-brain ~/Dev/agentic-portfolio` as a pre-flight gate for the Brain RAG indexer.
Phase 2 blocks F → I drive this in sequence. Blocks F, G, and H are all done. Block I is the
last Phase 2 block: wire `BrainValidator` into the CLI as a `validate-brain` subcommand and
add a global `--json` output flag for the RAG indexer.

## Completed this session

- **Block 2.H — Brain OKF frontmatter validator — DONE** (`24b6996`)
  - `OkfFrontmatter` serde struct in `src/brain/okf.rs` (all fields `Option`; `layer` as
    `Option<Vec<String>>`; extras tolerated via `#[serde(flatten)]`/deny_unknown = false)
  - `validate_md_file` entry point: read → extract frontmatter → parse YAML → field-check
    pipeline; short-circuits on missing/malformed frontmatter
  - Vocab helpers: `is_valid_layer`, `is_valid_project`, `is_valid_status` (closed sets from D27)
  - `BrainValidator` assembled in `src/brain/mod.rs` as a `ContentValidator` impl
  - Re-exports added to `src/lib.rs`
  - 30 unit tests in `src/brain/okf.rs` + 14 integration tests in `tests/brain_okf.rs`
  - **142 total tests pass** (91 unit + 51 integration); all 4 harness gates green; PASS on first review attempt
- **Close-out** (`d31b6a7`)
  - README directory map updated to include `okf.rs (OkfFrontmatter, validate_md_file)` in the `src/brain/` entry
  - Full validation suite re-verified; emoji gate clean; doc health sweep: no stale sections found
  - `docs/harness-json.md` flagged NEEDS_REVIEW (missing file referenced by 3 workflow docs — non-blocking)

## Remaining work

- **Block 2.I — `validate-brain` subcommand + `--json` flag** (start here — spec needed)
  - Add `validate-brain <path>` subcommand to the clap CLI (`src/main.rs`)
  - Wire it to `BrainValidator::run()` (already implemented in `src/brain/mod.rs`)
  - Add a global `--json` flag; when set, serialize `Report` as JSON to stdout instead of ANSI text
  - Exit code 1 on any error-severity diagnostic; 0 on clean/warn-only
- **Block D / E** (Phase 1 learn-ai) — deprioritized; do not start until Phase 2 is complete
- **NEEDS_REVIEW (non-blocking):** `docs/harness-json.md` is referenced from
  `docs/workflows/index.md` (lines 123 and 187) but does not exist; create when time allows

## Open questions / choices

None — clear to proceed. The `validate-brain` subcommand scope is fully settled in the
master-plan Block I definition. `--json` serialization format should mirror the existing `Report`
struct (errors/warnings with file + locator + message); exact shape can be decided during
implementation if it's not already in the master-plan spec.

## Context the next agent needs

- Repo is on `main`; no open worktrees. No GitHub remote — `gh pr list` will fail.
- Harness gates: `cargo fmt --check && cargo clippy -- -D warnings && cargo test && cargo build --release`
- **142 tests currently passing** (91 unit + 51 integration). Keep all green after every block.
- Source layout after 2.H:
  - `src/brain/crawl.rs` — `MdFile`, `crawl_brain`
  - `src/brain/okf.rs` — `OkfFrontmatter`, `validate_md_file`, vocab helpers
  - `src/brain/mod.rs` — `BrainValidator` (ContentValidator impl)
  - `src/learn_ai/` — LearnAiValidator
  - `src/validator.rs` — `ContentValidator` trait
  - `src/lib.rs` — re-exports both validators + core types
  - `src/main.rs` — clap CLI (currently only `validate` subcommand for learn-ai)
- The `layer` field in the live Brain corpus is always a YAML list (not scalar) — serde struct
  already models it as `Option<Vec<String>>`; no coercion path needed

## First command after `/prime`

`/generate-tasks 2.I-validate-brain-subcommand`
