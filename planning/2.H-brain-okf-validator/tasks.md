# Task Spec — Phase 2, Block H

**Status:** Done · **Last run:** 2026-06-26

## Goal
Add an `OkfFrontmatter` serde struct and an OKF frontmatter validator that checks required fields, controlled-vocab membership, kebab-case `doc_id`, and keyword count on every brain `.md` file, assembled behind a `BrainValidator` that ties Block G's crawl to these checks.

## Context Pointers
- **Plan section:** `planning/master-plan.md` → Phase 2 → *Block H — Brain OKF frontmatter validator*. That block's "What" and acceptance bar are authoritative.
- **Canonical OKF schema (mirror these rules exactly):** `~/Dev/agentic-portfolio/docs/okf-frontmatter.md` (brain decision D27). The three closed vocabularies, settled below against the live corpus:
  - **`layer` — controlled, LIST.** Empirically confirmed: every live `layer:` value is a YAML list (`[brain, meta]`, `[business]`, …), never a scalar. **The open scalar-vs-list question is settled: model `layer` as `Option<Vec<String>>`.** Closed set: `brain · engine · factory · console · surface · infra · business · content · meta`.
  - **`project` — controlled, scalar.** Closed set: `bastion · bastion-ui · python-orchestration · learn-ai · rag-engine-rs · claude-sdk-rs · workflow-engine-rs · markdown-engine-validator · bella · price-scout · amistad · base-template · brain`. Omitted entirely on cross-cutting docs — absence is **not** an error.
  - **`status` — controlled, scalar.** Closed set: `active · draft · deprecated · superseded · archived`.
- **Pattern to mirror:** `src/learn_ai/meta.rs` — the serde-`Option`-everything struct (extras tolerated, no `deny_unknown_fields`), per-field `require_str`/`missing` precise-locator diagnostics, `extract_frontmatter` → `serde_yaml::from_str` flow with single-error short-circuits for missing/malformed frontmatter. Reuse `crate::shared::{extract_frontmatter, is_kebab_case, non_empty}`.
- **Crawl + trait surface:** `src/brain/crawl.rs` provides `MdFile { path, rel, stem }` + `crawl_brain(root)` (Block G, done). `src/validator.rs` defines the `ContentValidator` trait (`type Item; crawl; validate_item; run`). `BrainValidator` is its second impl (after `LearnAiValidator`): `type Item = MdFile`, `crawl` delegates to `crawl_brain`, `validate_item` runs the OKF checks.
- **Standing rules:** every block ships with tests (CLAUDE.md rule 1); all four harness gates stay green.
- **Scope boundary (from the plan):** Block H is the OKF validator + its assembly into a runnable `BrainValidator`. **No** `validate-brain` subcommand and **no** `--json` flag — those are Block I. Do not touch `src/main.rs` or the learn-ai validators.

## Step-by-Step Tasks

### 1. `OkfFrontmatter` struct + frontmatter read/parse skeleton
- Create `src/brain/okf.rs`. Define `pub struct OkfFrontmatter` with all OKF fields as `Option`, extras tolerated (no `deny_unknown_fields`, mirroring `MdxFrontmatter`): `type_` (`#[serde(rename = "type")]`), `title`, `description`, `doc_id`, `layer: Option<Vec<String>>`, `project`, `status`, `keywords: Option<Vec<String>>`. (`related`/`timestamp` may be included as tolerated `Option` fields but are not validated here.)
- Add `pub fn validate_md_file(mf: &MdFile) -> Vec<Diagnostic>`: read `mf.path` (surface a read failure as a single `error` diagnostic located at `mf.rel`, mirroring `meta::read_content`); `extract_frontmatter` — **no frontmatter block → a single `error` diagnostic** (`locator: "frontmatter"`); `serde_yaml::from_str` — malformed YAML → a single `error` diagnostic. Field checks land in Task 2.
- Wire `pub mod okf;` into `src/brain/mod.rs`.
- Files: `src/brain/okf.rs` (new), `src/brain/mod.rs` (modified).

### 2. Required-field, controlled-vocab, doc_id, and keyword checks
- Depends on Task 1. Owns `src/brain/okf.rs`.
- **Required fields** (`type`, `title`, `description`): each absence is its own `error` diagnostic with a precise locator (`type`/`title`/`description`), via a `require_str`/`missing` helper pair like `meta.rs`. `type` is **presence-only** (open vocab — never check its value).
- **Controlled vocab** (only when the field is present): `layer` is a list — flag **each** member not in the layer set (locator `layer`); `project` scalar not in the project set → `error` (locator `project`); `status` scalar not in the status set → `error` (locator `status`). Absent `project`/`status`/`layer` is not an error (only `type`/`title`/`description` are required).
- **`doc_id`** (only when present): non-kebab-case → `error` (locator `doc_id`), using `crate::shared::is_kebab_case`.
- **`keywords`** (only when present): count outside 3–7 → a **`warning`** (not error), locator `keywords`. Absent `keywords` is not flagged.
- Add small testable vocab helpers (e.g. `is_valid_layer`, `is_valid_project`, `is_valid_status`) holding the closed sets from the Context Pointers.
- Files: `src/brain/okf.rs` (modified).

### 3. Assemble `BrainValidator` (ContentValidator impl) + re-export
- Depends on Task 2. Define `pub struct BrainValidator` implementing `crate::ContentValidator` with `type Item = MdFile`, `crawl` delegating to `crawl_brain`, and `validate_item` delegating to `okf::validate_md_file`. Place it in `src/brain/mod.rs` (mirrors how `LearnAiValidator` is exposed from `src/learn_ai/mod.rs`).
- Re-export from `src/lib.rs`: `pub use brain::BrainValidator;` and `pub use brain::okf::{OkfFrontmatter, validate_md_file};` (mirroring the existing `learn_ai` re-exports). Do **not** add a CLI subcommand (Block I).
- Files: `src/brain/mod.rs` (modified), `src/lib.rs` (modified — re-exports only).

### 4. Fixtures + unit/integration tests
- Depends on Tasks 2–3. Adds a `#[cfg(test)] mod tests` to `src/brain/okf.rs` and a new integration test file `tests/brain_okf.rs` (sibling to `tests/meta.rs`).
- **Unit tests** (in `okf.rs`): the vocab helpers accept every in-set value and reject an out-of-set value; a good frontmatter string is clean; each of the per-field rules fires its expected locator/severity when fed a crafted YAML string (good doc; missing `type`/`title`/`description` each; bad `layer` member; bad `project`; bad `status`; non-kebab `doc_id`; `keywords` count <3 and >7 both warn; missing-frontmatter → single error; malformed YAML → single error).
- **Integration tests** (`tests/brain_okf.rs`): write `.md` fixtures to a temp dir (same temp-dir style as `tests/meta.rs`) and drive them through `BrainValidator` (or `validate_md_file` on an `MdFile`) — assert a fully-good OKF doc is clean and at least one violation fixture produces the expected diagnostic end-to-end.
- Files: `src/brain/okf.rs` (modified — test module), `tests/brain_okf.rs` (new).

### 5. Validate
- Run the Validation Commands listed below and confirm all pass.

## Acceptance Criteria
- `OkfFrontmatter` exists with all fields `Option`, `layer` typed as `Option<Vec<String>>`, extras tolerated; re-exported from `src/lib.rs` along with `validate_md_file` and `BrainValidator`.
- Missing `type`, `title`, or `description` each emits its own `error` diagnostic at the matching locator; `type` value is never vocab-checked.
- A `layer` member, `project`, or `status` value outside its closed set emits an `error` at that field's locator; an absent `project`/`status`/`layer` does not.
- A non-kebab `doc_id` emits an `error` at `doc_id`; `keywords` with fewer than 3 or more than 7 entries emits a `warning` at `keywords`.
- A file with no frontmatter block emits exactly one `error`; malformed YAML emits exactly one `error`.
- `BrainValidator` implements `ContentValidator` (crawl = `crawl_brain`, validate = OKF checks) and runs end-to-end via the trait's `run` driver.
- New unit tests cover every rule and vocab helper; new integration tests drive fixtures through `BrainValidator`/`validate_md_file`.
- The existing learn-ai and Block G crawl tests are unchanged and still pass; all four harness gates pass.

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes
- **Settled (handoff open question):** `layer` is a list, not a scalar — confirmed against the live corpus (`grep -rh '^layer:' ~/Dev/agentic-portfolio/docs ~/Dev/agentic-portfolio/planning` returns only `[...]` forms) and the canonical schema. No scalar-coercion handling is needed.

## Amendment Log
<!-- Append-only. Pipeline stages append one dated line here when they deviate from the spec. -->
_No amendments yet._
