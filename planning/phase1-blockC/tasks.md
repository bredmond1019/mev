# Task Spec — Phase 1, Block C — Frontmatter & JSON struct validation

## Goal
Deserialize each module `.json` into a strict `ModuleMeta`, path `metadata.json` into its own
struct, and parse MDX frontmatter as real YAML — emitting a `Diagnostic` for every missing
required field, bad enum value, or malformed format.

## Context Pointers
- **Plan:** `planning/master-plan.md` → *Phase 1 → Block C — Frontmatter & JSON struct validation*.
  Also the Phase 1 preamble: the bar is a *superset of* `learn-ai/scripts/validate-content.ts`
  (D2); the universal currency is the `Diagnostic` (`error` → exit 1, `warning` → exit 0); only
  the reporter prints.
- **Decisions:** `planning/decisions/D2-scope-and-sequence.md` (superset bar; Diagnostic currency).
- **Repo files (extend, do not rewrite):**
  - `src/lib.rs` — `Diagnostic` / `Severity` / `Report` / `validate()`. `validate()` already calls
    `crawl()` and collects filename diagnostics; Block C appends struct/frontmatter diagnostics.
  - `src/crawl.rs` — `Corpus`, `ContentFile`, `FileKind` (`LearnModuleJson`, `PathMetadataJson`,
    `ModuleMdx`), `Locale`. Each `ContentFile` carries `path` (absolute, readable) and `rel`
    (for diagnostic locators). Iterate `corpus.files` and dispatch on `kind`.
  - `tests/crawl.rs` — existing integration-test style (temp-dir fixture trees); mirror it.
- **CLAUDE.md standing rules:** every block ships with tests (rule 1); maintain OKF frontmatter;
  all four harness gates must stay green (`fmt`, `clippy -D warnings`, `test`, `build`).
- **Schema reference:** `../learn-ai/content/learn/schemas/module-schema.json` — mirror its enums
  where practical (see enum values inlined in Task 2 below).
- **Dependencies:** `serde`, `serde_json`, and `serde_yaml` are already in `Cargo.toml` — no new
  deps expected.

## Step-by-Step Tasks

### 1. Add a `validate` (struct/frontmatter) module
- Add `src/meta.rs` (or `src/validate_meta.rs`), re-exported from `lib.rs`, holding the
  serde structs and the per-file validation functions. Keep `crawl.rs` focused on the walk.
- Read each file's contents from `ContentFile.path`; surface read/parse failures as an
  `error`-severity `Diagnostic` (do not panic, do not abort the run).

### 2. Define and validate the `ModuleMeta` struct (`FileKind::LearnModuleJson`)
- Deserialize the module `.json` with serde into a struct whose `metadata` requires:
  `id, pathId, title, description, duration, type, difficulty, order, objectives, tags, version,
  lastUpdated`, plus a non-empty `sections[]` where each section requires `id, type, order`.
  Use `serde(deny_unknown_fields)` sparingly — the live files carry extra keys (`author`,
  `estimatedCompletionTime`, `prerequisites`, section `title`/`estimatedDuration`/`content`), so
  do **not** deny unknown fields; only enforce the required set.
- A missing required field → one `error` diagnostic with a precise locator
  (e.g. `metadata.duration`, `sections[2].id`).
- Validate formats/enums after a successful parse:
  - `metadata.id` kebab-case: `^[a-z0-9]+(-[a-z0-9]+)*$`.
  - `metadata.duration` format: `^\d+\s+(minutes?|hours?)$`.
  - `metadata.difficulty` ∈ `beginner | intermediate | advanced`.
  - `metadata.type` ∈ `theory | concept | practice | project | assessment`.
  - section `type` ∈ `content | quiz | exercise | project | assessment`.
  - non-empty `sections[]` (at least one section).
- Implement regex-equivalent checks by hand (no `regex` crate), matching the `is_valid_*` helper
  style already in `crawl.rs`.

### 3. Define and validate path `metadata.json` (`FileKind::PathMetadataJson`)
- Deserialize into a struct requiring: `id, title, description, level, duration, version,
  lastUpdated, topics, modules`. Tolerate extra keys (`difficulty`, `totalDuration`, `author`,
  `prerequisites`, `outcomes`, `resources`).
- Missing required field → one `error` diagnostic with locator (e.g. `level`, `modules`).
- (Cross-referencing `modules[]` against real files is **Block D**, not here — validate structure
  and presence only.)
- **`level` must be validated case-insensitively.** Live `metadata.json` files use capitalized
  values (`"Intermediate"`, `"Beginner"`, `"Advanced"`); the TS validator already lowercases
  before the enum check. Match `s.to_lowercase().as_str()` against
  `"beginner" | "intermediate" | "advanced"` — do not reject capitalised values.

### 4. Parse and validate MDX frontmatter as real YAML (`FileKind::ModuleMdx`)
- Extract the leading `---\n … \n---` frontmatter block (between the first two `---` fences) and
  parse it with `serde_yaml` — **not** substring matching.
- Require: `title, description, duration, difficulty, lastUpdated`. Missing/empty → `error`
  diagnostic with locator (e.g. `frontmatter.duration`).
- Validate `difficulty` enum and `duration` format the same way as the JSON path (factor the
  shared checks into helpers).
- A file with no frontmatter block, or an unterminated/ malformed-YAML block, → one `error`
  diagnostic rather than a panic.

### 5. Wire the checks into `validate()`
- In `validate()` (or a helper it calls), after the crawl, iterate `corpus.files` and dispatch
  each file to the matching validator by `kind`, appending all diagnostics to the `Report`.
- Preserve `validate()`'s public contract (returns a `Report` driving the exit code) so `main.rs`
  is untouched. Filename diagnostics from Block B must still appear.

### 6. Tests against fixtures
- Extend `tests/` (new `tests/meta.rs` or additions to `tests/crawl.rs`) with temp-dir fixtures:
  - a **good** module `.json` + `metadata.json` + `.mdx` (all required fields, valid enums/formats)
    → zero error diagnostics.
  - **broken** variants, each asserting the expected diagnostic: missing `duration`; bad
    `difficulty` enum; non-kebab `id`; malformed `duration` string; empty `sections[]`; a section
    missing `id`; `metadata.json` missing `modules`; an MDX file missing frontmatter; an MDX file
    missing a required frontmatter key; an MDX file with malformed YAML.
- Assert locators and severities are exactly as specified — not just the count.
- Keep `smoke.rs` and the Block B `crawl.rs` tests green (empty tree → clean report).

### 7. Validate
- Run the Validation Commands listed below and confirm all pass.
- Optionally run `cargo run -- validate ../learn-ai/content/learn` if the sibling checkout exists,
  to confirm the live corpus parses (expect it to be clean, or to surface only genuine issues).

## Acceptance Criteria
- Module `.json` files deserialize into a strict `ModuleMeta`; every missing required field
  (`id, pathId, title, description, duration, type, difficulty, order, objectives, tags, version,
  lastUpdated`, plus non-empty `sections[]` with `id/type/order`) emits the expected diagnostic.
- Enum violations (`difficulty`, module `type`, section `type`) and format violations
  (kebab-case `id`, `duration` `^\d+\s+(minutes?|hours?)$`) each emit the expected diagnostic.
- Path `metadata.json` files require `id, title, description, level, duration, version,
  lastUpdated, topics, modules`; each missing field emits the expected diagnostic.
- MDX frontmatter is parsed as real YAML and requires `title, description, duration, difficulty,
  lastUpdated`; missing block, missing key, and malformed YAML each emit an error (no panic).
- New fixture-driven tests cover good + each deliberately-broken case and pass; existing Block B
  and smoke tests stay green.
- All four harness gates are green.

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes

**`author` field (conscious omission):** `ModuleMeta` does not require `author`, matching the TS
validator's behaviour. The JSON schema (`content/learn/schemas/module-schema.json`) lists `author`
as required, and every live module file carries `"author": "Brandon J. Redmond"`, but since the
goal is a superset of the TS validator (not full JSON-schema enforcement), omitting it is
intentional and consistent. If enforcement is ever wanted, add `pub author: Option<String>` to
`ModuleMeta` and call `require_str(cf, "metadata.author", &meta.author, diags)` in
`validate_module_metadata` — a two-line change.
