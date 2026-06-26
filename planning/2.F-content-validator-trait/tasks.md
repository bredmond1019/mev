---
type: Spec
title: Phase 2, Block F — ContentValidator trait + shared core
description: Introduce the associated-type ContentValidator trait, extract shared helpers into a shared module, and relocate the learn-ai code behind a LearnAiValidator — preserving the public API and the full existing test suite.
doc_id: 2-F-content-validator-trait
layer: [engine]
project: markdown-engine-validator
status: active
keywords: [content-validator, trait, refactor, shared-core, learn-ai, module-layout]
related: [master-plan]
---

# Task Spec — Phase 2, Block F — ContentValidator trait + shared core

**Status:** Not started · **Last run:** never

## Goal
Introduce the associated-type `ContentValidator` trait + a `shared` helper module, and relocate the learn-ai code behind a `LearnAiValidator`, rewriting `validate()` as a thin wrapper while preserving the public API so the full existing test suite passes unchanged.

## Context Pointers
- `planning/master-plan.md` — **Phase 2 → Block F** (the block being decomposed) and the Phase 2 preamble ("one core, many schemas"; each block must keep the existing learn-ai tests passing). Block G (Brain crawl) and Block H (OKF validator) are the first consumers the trait must accommodate, but are **out of scope here**.
- `planning/master-plan.md` → "Architecture / Design Overview" — the `ContentValidator` is an **associated-type trait** (`type Item; crawl(); validate_item(); run()` driver); `main.rs` selects the concrete validator per subcommand, so **static dispatch** suffices. The generic core stays free of any consumer's domain types.
- **Existing code (the refactor surface):**
  - `src/lib.rs` — owns `Severity`, `Diagnostic`, `Report`, `validate()`, and the `pub use` re-exports. `validate()` currently calls `crawl::crawl()` then loops `meta::validate_file`. Tests reach the public API through these re-exports (`mev::{ContentFile, Corpus, FileKind, Locale, crawl, validate_file, validate, Diagnostic, Severity}`).
  - `src/crawl.rs` — `Corpus`, `ContentFile`, `FileKind`, `Locale`, `crawl()`; learn-ai-specific (`paths/<id>/modules/...` derivation). Moves into `learn_ai/`.
  - `src/meta.rs` — `validate_file()` + the helpers to extract: `extract_frontmatter` (line ~355), `is_kebab_case` (~472), `non_empty` (~464), each with unit tests in its `#[cfg(test)] mod tests`. Moves into `learn_ai/`; the three helpers move to `shared`.
  - `src/main.rs` — calls `mev::validate(&path)`. **Must not change** (its contract is preserved).
  - `tests/crawl.rs`, `tests/meta.rs`, `tests/smoke.rs` — import only from the `mev` crate root; **must pass unchanged with no signature changes**.
- **Standing rules:** tests ship with every block; no `regex` crate (str methods only); decisions append-only.

## Step-by-Step Tasks

### 1. Extract shared helpers into `src/shared.rs`
- Create `src/shared.rs` and declare `mod shared;` in `src/lib.rs`.
- Move `extract_frontmatter`, `is_kebab_case`, and `non_empty` from `src/meta.rs` into `src/shared.rs`, making them `pub(crate)` (or `pub`) so `meta`/`learn_ai` can call them.
- Move each helper's existing unit tests (currently in `src/meta.rs`'s `#[cfg(test)] mod tests`, e.g. `extract_frontmatter_helper`) into a `#[cfg(test)] mod tests` in `src/shared.rs` **verbatim** (no assertion changes).
- Update `src/meta.rs` to import the helpers from `crate::shared` (`use crate::shared::{extract_frontmatter, is_kebab_case, non_empty};`) and delete the moved definitions + their moved tests.
- **Files:** `src/shared.rs` (new), `src/meta.rs` (modify), `src/lib.rs` (add `mod shared`).
- **Acceptance:** `cargo test` green; the moved helper tests now run from `shared`; no behavior change.
- Depends on: none.

### 2. Define the `ContentValidator` trait in `src/validator.rs`
- Create `src/validator.rs` and declare `mod validator;` in `src/lib.rs`; re-export the trait (`pub use validator::ContentValidator;`).
- Define the associated-type trait, generic over the consumer's item type and free of any learn-ai domain type:
  ```rust
  pub trait ContentValidator {
      /// The per-item unit this validator crawls and checks (e.g. a ContentFile).
      type Item;
      /// Walk `root`, returning the items to validate plus any crawl-time diagnostics.
      fn crawl(&self, root: &std::path::Path) -> (Vec<Self::Item>, Vec<Diagnostic>);
      /// Validate a single crawled item.
      fn validate_item(&self, item: &Self::Item) -> Vec<Diagnostic>;
      /// Default driver: crawl, then validate each item, collecting all diagnostics into a Report.
      fn run(&self, root: &std::path::Path) -> Report {
          let (items, mut diagnostics) = self.crawl(root);
          for item in &items {
              diagnostics.extend(self.validate_item(item));
          }
          Report { diagnostics }
      }
  }
  ```
  (Final signatures may adjust to fit the existing `crawl`/`Corpus` shapes — the load-bearing requirement is: associated `Item`, `crawl` + `validate_item` methods, and a **default `run`** driver. Keep it static-dispatch friendly; no `dyn`.)
- Add a unit test in `src/validator.rs` that exercises the default `run` driver with a tiny stub validator (an inline test-only impl whose `Item = ()`), asserting `run` collects crawl diagnostics + per-item diagnostics into the `Report`.
- **Files:** `src/validator.rs` (new), `src/lib.rs` (add `mod validator` + re-export).
- **Acceptance:** trait compiles; the stub-driver unit test passes; `cargo clippy -- -D warnings` clean.
- Depends on: 1.

### 3. [~] Relocate the learn-ai code into a `src/learn_ai/` module + implement `LearnAiValidator`
- Create the `src/learn_ai/` module directory with `src/learn_ai/mod.rs`.
- Move `src/crawl.rs` → `src/learn_ai/crawl.rs` and `src/meta.rs` → `src/learn_ai/meta.rs` **verbatim** (adjust only `crate::` paths that changed — e.g. helper imports now resolve from `crate::shared`, and `crate::Diagnostic` is unchanged). Declare both as submodules in `src/learn_ai/mod.rs`.
- In `src/learn_ai/mod.rs`, define `pub struct LearnAiValidator;` and `impl ContentValidator for LearnAiValidator` with `type Item = ContentFile`, wiring `crawl` to the existing `crawl::crawl` (returning `corpus.files` + the crawl diagnostics) and `validate_item` to `meta::validate_file`.
- In `src/lib.rs`, replace `mod crawl; mod meta;` with `mod learn_ai;`, and update the re-exports so the public surface is unchanged: `pub use learn_ai::crawl::{ContentFile, Corpus, FileKind, Locale, crawl};` and `pub use learn_ai::meta::validate_file;` (plus `pub use learn_ai::LearnAiValidator;`).
- Keep all moved `#[cfg(test)] mod tests` blocks intact so the in-module unit tests still run from their new location.
- **Files:** `src/learn_ai/mod.rs` (new), `src/learn_ai/crawl.rs` (moved from `src/crawl.rs`), `src/learn_ai/meta.rs` (moved from `src/meta.rs`), `src/lib.rs` (swap module decls + re-exports). Deletes `src/crawl.rs`, `src/meta.rs`.
- **Acceptance:** `cargo build --release` green; `mev::{ContentFile, Corpus, FileKind, Locale, crawl, validate_file}` still resolve from the crate root; moved unit tests pass.
- Depends on: 1, 2.

### 4. Rewrite `validate()` as a thin wrapper over `LearnAiValidator`
- In `src/lib.rs`, rewrite `pub fn validate(root: &Path) -> anyhow::Result<Report>` to delegate to the trait: construct a `LearnAiValidator` and return `Ok(LearnAiValidator.run(root))` (or the equivalent that preserves the current crawl-then-validate ordering and the exact same diagnostics).
- The signature, return type, and observable behavior of `validate()` are **unchanged** — `src/main.rs` is not modified.
- Confirm the `crawl`/`validate_file` re-exports remain so `tests/crawl.rs` and `tests/meta.rs` compile and pass with **no edits**.
- **Files:** `src/lib.rs` (modify `validate()` body only).
- **Acceptance:** `tests/smoke.rs`, `tests/crawl.rs`, `tests/meta.rs` pass unchanged; `validate()` produces the identical diagnostic set as before the refactor.
- Depends on: 3.

### 5. Validate
- Run the Validation Commands listed below and confirm all four pass.
- Confirm no public signature changed: `git diff` touches no test file under `tests/` and does not modify `src/main.rs`.
- Optionally run `cargo run -- validate ../learn-ai/content/learn` if the sibling checkout exists, to confirm the live corpus result is identical to pre-refactor.

## Acceptance Criteria
- A `ContentValidator` trait exists with an associated `Item` type, `crawl` + `validate_item` methods, and a default `run` driver; it carries no learn-ai domain type.
- `extract_frontmatter`, `is_kebab_case`, and `non_empty` live in a `shared` module with their unit tests, and `meta` consumes them from there.
- The learn-ai code lives under `src/learn_ai/` and is reachable behind a `LearnAiValidator: ContentValidator` impl.
- `validate()` is a thin wrapper delegating to `LearnAiValidator` with its signature and behavior unchanged; `src/main.rs` is not modified.
- The public crate surface (`mev::{ContentFile, Corpus, FileKind, Locale, crawl, validate_file, validate, Diagnostic, Severity}`) is preserved via `pub use`.
- The full existing test suite (`tests/crawl.rs`, `tests/meta.rs`, `tests/smoke.rs` + in-module unit tests) passes with **no edits to any test file**.
- All four harness gates pass.

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes
<filled in as work happens>

## Amendment Log
<!-- Append-only. Pipeline stages append one dated line here when they deviate from the spec. -->
_No amendments yet._
