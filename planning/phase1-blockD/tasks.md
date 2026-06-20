---
type: Spec
title: Phase 1, Block D — Cross-file integrity
description: Pair existence, anchor-slice contract, ID coherence, and callout-type validation.
---

# Phase 1, Block D — Cross-file integrity (the differentiator)

## Goal

Add `src/integrity.rs` containing `validate_integrity(corpus: &Corpus) -> Vec<Diagnostic>` and
wire it into `validate()`. The checks cover: (1) pair existence — every module `.json` must have
a sibling `.mdx` and vice versa; (2) the anchor-slice contract — each section whose
`content.source = "<file>.mdx#<anchor>"` must resolve to a heading containing `{#<anchor>}` in
the target `.mdx`; (3) ID coherence — `metadata.json.modules[]` must map to real files and
`sections[].id` must match the anchor in `content.source`; (4) callout types in `.mdx` body
content must be `info|warning|success|error`. A renamed anchor in a fixture is flagged here while
the TypeScript validator stays silent.

## Context Pointers

- `planning/master-plan.md` — Phase 1 → Block D specification and the Phase 1 preamble (superset
  of `learn-ai/scripts/validate-content.ts`; `Diagnostic` is the universal currency; only the
  reporter prints; every error → exit 1).
- `planning/decisions/D2-scope-and-sequence.md` — superset bar; `Diagnostic` currency.
- `planning/context.md` — governing principles (tests ship with every block; no fabricated data).
- **Existing code to extend (do not rewrite):**
  - `src/lib.rs` — `validate()` calls `crawl()` and then dispatches per-file checks via
    `meta::validate_file`; Block D appends a call to `integrity::validate_integrity(&corpus)`.
  - `src/crawl.rs` — `Corpus`, `ContentFile`, `FileKind`, `Locale`. The corpus already carries
    `path_id`, `module_id` (stem), `locale`, and absolute `path`/relative `rel` per file.
    `Corpus::modules_for(path_id, locale)` returns module files for a path+locale pair.
  - `src/meta.rs` — `validate_file` per-file checks (struct/frontmatter). Block D is cross-file
    only; do not duplicate per-file logic here.
  - `tests/crawl.rs` and `tests/meta.rs` — mirror the temp-dir fixture style.
- **Site anchor regex reference:** the site uses `{#<anchor>}` in MDX headings, e.g.
  `## Overview {#overview}`. The validator must detect this pattern without the `regex` crate,
  matching the approach in `crawl.rs` (character-level or `contains`/`find`-based helpers).

## Step-by-Step Tasks

### 1. Add `src/integrity.rs` — module scaffold and public API

- Create `src/integrity.rs` and declare it in `src/lib.rs` (`mod integrity;`).
- Define the public entry point:
  ```
  pub fn validate_integrity(corpus: &Corpus) -> Vec<Diagnostic>
  ```
  This function will orchestrate all cross-file checks (Tasks 2–5) and collect their diagnostics.
- Do not implement check logic in this task — just establish the module, the function signature,
  and an empty body returning `vec![]`.
- Wire the call into `validate()` in `src/lib.rs`:
  ```rust
  diagnostics.extend(integrity::validate_integrity(&corpus));
  ```
  (After the `meta::validate_file` loop.) Preserve `validate()`'s public contract; `main.rs` must
  not change.
- The existing test suite must remain green after this task (no behavior change yet).
- Depends on: none.

### 2. Pair existence check

- For each `path_id` in `corpus.path_ids()`, and for each `locale` in `[Locale::En, Locale::PtBr]`:
  - Collect all `LearnModuleJson` files and all `ModuleMdx` files with that `path_id` and `locale`,
    keyed by `module_id` stem (the `ContentFile.module_id` field, which the crawler sets to
    `Some("NN-slug")` for module files).
  - For every `.json` stem without a matching `.mdx` stem → emit:
    ```
    Diagnostic::error(cf.rel, "pair", "module JSON has no matching .mdx sibling")
    ```
  - For every `.mdx` stem without a matching `.json` stem → emit:
    ```
    Diagnostic::error(cf.rel, "pair", "module MDX has no matching .json sibling")
    ```
  - Files in `Locale::PtBr` are checked independently of `Locale::En` (a pt-BR `.json` must have
    a pt-BR `.mdx` sibling; presence of the EN pair does not satisfy the pt-BR requirement).
  - `PathMetadataJson` files are excluded from pair checking.
- Depends on: 1.

### 3. Anchor-slice contract

- For each `LearnModuleJson` file in the corpus:
  - Read the file's content from disk (surface IO failure as an `error` diagnostic; do not panic).
  - Deserialize the JSON. If deserialization fails, skip anchor checks for this file (Block C
    already emits the parse-error diagnostic).
  - For each section `i` in `sections[]` where the section has a `content.source` string of the
    form `"<filename>.mdx#<anchor>"`:
    - Parse `<filename>` and `<anchor>` from the source string (split on `#`; the part before `#`
      is the filename, the part after is the anchor).
    - Locate the sibling `.mdx` `ContentFile` in the corpus: same `path_id`, same `locale`,
      `module_id` stem == `<filename>` without extension. If not found:
      ```
      Diagnostic::error(json_cf.rel, format!("sections[{i}].content.source"),
          format!("source file '{filename}' not found in corpus"))
      ```
    - If the `.mdx` file is found, read it from disk. If unreadable, emit an `error` diagnostic
      and skip the heading check.
    - Scan the `.mdx` body for a heading containing `{#<anchor>}`. The check must replicate the
      site's heading-anchor pattern: a line that contains `{#<anchor>}` anywhere in it. Helper
      `fn heading_has_anchor(mdx: &str, anchor: &str) -> bool` using `lines().any(...)` and
      `contains` — no `regex` crate.
    - If no heading with the anchor is found:
      ```
      Diagnostic::error(json_cf.rel, format!("sections[{i}].content.source"),
          format!("anchor '{anchor}' not found in '{filename}'"))
      ```
- Depends on: 1.

### 4. ID coherence checks

Four sub-checks, all in `validate_integrity`:

**4a. `metadata.json.modules[]` → real files**
- For each `PathMetadataJson` file in the corpus:
  - Read and parse the JSON. Skip if unreadable or unparseable (Block C already covers that).
  - The `modules` field is an array. Each element may be a string (module stem or file ref) or an
    object. Focus on string entries and entries with a `"id"` or `"file"` key containing the
    module stem.
    - Strategy: deserialize `modules` as `Vec<serde_json::Value>`; for each value, extract the
      stem string (string value directly, or `v["id"].as_str()` or `v["file"].as_str()`).
  - For each extracted stem `s`: check whether the corpus contains a `LearnModuleJson` file with
    `path_id` == this path's `path_id`, `locale == En`, and `module_id` stem ending with `-<s>`
    or equal to `s`. If not found:
      ```
      Diagnostic::error(metadata_cf.rel, "modules",
          format!("module '{s}' listed in modules[] has no corresponding file"))
      ```
  - Note: live `metadata.json` files use module IDs (without numeric prefix) while filenames use
    `NN-slug`. Match on the stem suffix (after the numeric prefix and dash). This is a best-effort
    check — emit a warning, not an error, for no-match if the match heuristic is uncertain.

**4b. `metadata.id` has no numeric prefix (informational)**
- For each `LearnModuleJson`, if the JSON deserializes successfully: verify that `metadata.id`
  does not start with digits followed by a dash (e.g. `"01-intro"` is wrong; `"intro"` is correct).
  The filename carries the numeric prefix; `metadata.id` must not. If violated:
    ```
    Diagnostic::error(cf.rel, "metadata.id",
        "metadata.id must not carry the filename numeric prefix (e.g. use 'intro', not '01-intro')")
    ```
  (Note: Block C already validates kebab-case; this is an additional constraint for the `id` field
  specifically.)

**4c. `sections[].id` == anchor in `content.source`**
- For each section that has both `id` and `content.source` set:
  - Parse the anchor from `content.source` (the part after `#`).
  - If `section.id` != anchor:
    ```
    Diagnostic::error(cf.rel, format!("sections[{i}].id"),
        format!("section id '{sid}' does not match anchor '{anchor}' in content.source"))
    ```

- Depends on: 1, 3 (shares JSON parse + source parsing logic; factor into helpers).

### 5. Callout type validation in `.mdx` body

- For each `ModuleMdx` file in the corpus:
  - Read the file content from disk (surface IO failure as error, do not panic).
  - Scan the body (the content after the closing `---` frontmatter fence) for callout blocks.
    The MDX callout syntax is `:::type` at the start of a line (where `type` is a word).
  - Helper: `fn extract_callout_types(body: &str) -> Vec<&str>` that returns each `type` token
    found on lines matching `^:::<word>` (no trailing space or args required; stop at the next
    non-word character after `:::`).
  - Valid callout types: `info | warning | success | error`.
  - For each callout type token not in the valid set:
    ```
    Diagnostic::error(cf.rel, "callout",
        format!("unknown callout type '{}'; must be info|warning|success|error", token))
    ```
- Depends on: 1.

### 6. Tests against fixtures

Add `tests/integrity.rs` with temp-dir fixtures covering:

**Pair existence:**
- Good: a `.json` + `.mdx` sibling with the same stem → zero error diagnostics.
- Orphan `.json` (no sibling `.mdx`) → error with locator `"pair"`.
- Orphan `.mdx` (no sibling `.json`) → error with locator `"pair"`.

**Anchor-slice contract:**
- Good: a section with `content.source = "01-intro.mdx#overview"` where the `.mdx` contains
  `## Overview {#overview}` → no errors.
- Bad anchor: the same setup but the `.mdx` uses `{#intro}` instead → error on
  `sections[N].content.source` with message containing `"anchor"`.
- Missing source file: `content.source` references a file not in the corpus → error.

**ID coherence:**
- `metadata.json.modules[]` containing a stem with no matching corpus file → error on `"modules"`.
- `sections[].id` != `content.source` anchor → error on `sections[N].id`.
- `metadata.id` with numeric prefix (`"01-intro"`) → error on `"metadata.id"`.

**Callout types:**
- Good: `:::info` in MDX body → no error.
- Bad: `:::tip` in MDX body → error with locator `"callout"`.

**Regression:**
- Confirm all Block B (`crawl.rs`) and Block C (`meta.rs`) tests remain green.
- An empty corpus → `validate_integrity` returns an empty `Vec` (no panic).

### 7. Validate

- Run the Validation Commands listed below and confirm all four pass.
- Optionally run `cargo run -- validate ../learn-ai/content/learn` if the sibling checkout
  exists, to confirm the live corpus is clean (or surfaces only genuine issues). This is not
  required for the pass verdict.

## Acceptance Criteria

- A `.json` module without a sibling `.mdx` (same stem, same locale) emits an `error` diagnostic
  with locator `"pair"`.
- An `.mdx` module without a sibling `.json` emits an `error` diagnostic with locator `"pair"`.
- A section whose `content.source` anchor does not exist in the target `.mdx` heading emits an
  `error` diagnostic — this is the case the TypeScript validator misses.
- A `metadata.json.modules[]` entry that has no matching corpus file emits an error.
- A `sections[].id` that differs from its `content.source` anchor emits an error.
- A `metadata.id` carrying a numeric prefix emits an error.
- An unknown callout type in `.mdx` body content emits an error with locator `"callout"`.
- All Block B (crawl) and Block C (meta) tests remain green.
- All four harness gates pass.

## Validation Commands

```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes

- **No `regex` crate.** All pattern matching uses character-level or `str` methods
  (`contains`, `find`, `starts_with`, `split`, `lines().any(...)`), consistent with the
  approach in `crawl.rs` and `meta.rs`.
- **Anchor detection scope.** The heading-anchor pattern `{#<anchor>}` can appear anywhere on a
  heading line. `heading_has_anchor` should check each line with `contains(format!("{{#{anchor}}}")`.
- **`modules[]` match heuristic.** Live `metadata.json` files list module IDs without numeric
  prefixes (e.g. `"intro-to-mcp"`), while filenames are `01-intro-to-mcp.json`. The match should
  check whether any `module_id` stem in the corpus ends with `-<id>` or equals `<id>`. Emit a
  `warning` (not `error`) if neither match succeeds and the heuristic may have false-positived.
- **pt-BR pair check.** Block D checks pt-BR pairs independently. Block E (pt-BR parity) will
  enforce that EN modules have a pt-BR mirror — that is out of scope here.
- **`content.source` parsing.** Only sections with a `content` object containing a `source`
  string in `"<file>#<anchor>"` form are checked. Sections without `content.source` are skipped
  silently.
- **Section deserialization for Block D.** The `ModuleSection` struct in `meta.rs` does not
  currently capture `content.source`. Block D must either extend `ModuleSection` (preferred, to
  keep all module-JSON structs in `meta.rs`) or define a local shadow struct in `integrity.rs`.
  Extending `ModuleSection` is the cleaner path; add `pub content: Option<SectionContent>` with
  `SectionContent { pub source: Option<String> }`.
