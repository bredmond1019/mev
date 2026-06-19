# Task Breakdown — Phase 1, Block B — Crawl & classify

## Source Spec
`planning/phase1-blockB/tasks.md`

## Goal
`walkdir` the content root, classify each file as `learn-module-json`, `path-metadata-json`, or
`module-mdx`, build a `Corpus` grouped by path-id / module-id, and surface filename-convention
violations as diagnostics.

## How to Use
Work top to bottom. Each sub-step is a single atomic action. Run the inline **Verify** checks as
you go — do not batch them at the end. Each check must pass before continuing.

---

## Ground truth (discovered from the live tree at `../learn-ai/content/learn`)

Relative-path shapes the classifier must handle (root = the content dir passed to `validate`):

| Relative path | `FileKind` | `path_id` | `module_id` | `locale` |
|---|---|---|---|---|
| `paths/mcp-fundamentals/metadata.json` | `PathMetadataJson` | `mcp-fundamentals` | `None` | `En` |
| `paths/mcp-fundamentals/modules/01-introduction-to-mcp.json` | `LearnModuleJson` | `mcp-fundamentals` | `01-introduction-to-mcp` | `En` |
| `paths/mcp-fundamentals/modules/01-introduction-to-mcp.mdx` | `ModuleMdx` | `mcp-fundamentals` | `01-introduction-to-mcp` | `En` |
| `paths/mcp-fundamentals/pt-BR/metadata.json` | `PathMetadataJson` | `mcp-fundamentals` | `None` | `PtBr` |
| `paths/mcp-fundamentals/pt-BR/modules/01-introduction-to-mcp.mdx` | `ModuleMdx` | `mcp-fundamentals` | `01-introduction-to-mcp` | `PtBr` |
| `schemas/module-schema.json`, `shared/templates/code-examples.json`, `CLAUDE.md`, `common-issues.md` | *(skipped)* | — | — | — |

**Derivation rule:** split the path relative to `root` into components. First component must be
`paths` (else skip — this excludes `schemas/`, `shared/`, top-level `*.md`). Second component is
`path_id`. If the next component is `pt-BR`, `locale = PtBr` and consume it; else `locale = En`.
Remaining tail is either `metadata.json` (→ `PathMetadataJson`) or `modules/<file>` (→ module
file, `module_id` = file stem). Anything else under a path (stray files) is skipped.

> **Scope note:** `locale` is **not** named in the spec's `ContentFile`, but it is required to
> derive `path_id` correctly (pt-BR nests inside the path dir) and Block E (pt-BR parity) consumes
> it. Adding it now is in-scope discovery, not scope creep. Locale *parity checks* remain Block E.

---

## Steps

### Step 1: Define classification + corpus types

#### 1.1 Create `src/crawl.rs` with the core enums and `ContentFile`
**File:** `src/crawl.rs` (new)
**Action:** create file with the type skeleton.
- Imports: `use std::collections::BTreeMap;`, `use std::path::{Path, PathBuf};`, `use crate::Diagnostic;`
- `Locale` enum: `#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum Locale { En, PtBr }`.
- `FileKind` enum: `#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum FileKind { LearnModuleJson, PathMetadataJson, ModuleMdx }`
  (no `Unknown` variant — non-content files are skipped during the walk, never constructed into a
  `ContentFile`).
- `ContentFile` struct, all fields `pub`:
  ```rust
  #[derive(Debug, Clone)]
  pub struct ContentFile {
      pub path: PathBuf,          // path as walked (joined on root)
      pub rel: PathBuf,           // path relative to the content root (for diagnostics/locators)
      pub kind: FileKind,
      pub path_id: String,
      pub module_id: Option<String>,
      pub locale: Locale,
  }
  ```
- Use `BTreeMap` (not `HashMap`) everywhere so corpus iteration order is deterministic — fixture
  tests assert on order and clippy/CI must be reproducible.

#### 1.2 Add the `Corpus` type and accessors to `src/crawl.rs`
**File:** `src/crawl.rs`
**Action:** append the corpus container below the types.
```rust
/// All content files, grouped by path-id then module-id. The metadata.json for a path
/// (module_id == None) is held separately per locale.
#[derive(Debug, Default)]
pub struct Corpus {
    pub files: Vec<ContentFile>,
}

impl Corpus {
    /// Every distinct path-id present, sorted (BTreeSet-backed).
    pub fn path_ids(&self) -> Vec<&str> { /* dedup + sort from self.files */ }

    /// All module files (json + mdx) for a given path-id and locale.
    pub fn modules_for<'a>(&'a self, path_id: &str, locale: Locale) -> Vec<&'a ContentFile> { /* filter */ }

    /// Look up a single content file by (path_id, module_id, locale, kind). Used by Block D pairing.
    pub fn get(&self, path_id: &str, module_id: &str, locale: Locale, kind: FileKind) -> Option<&ContentFile> { /* find */ }
}
```
Implement the bodies with plain iterator filters over `self.files`. Keep methods `pub` so
integration tests construct a `Corpus { files: vec![...] }` and inspect it.

**Verify:** `cargo check` → compiles (module not yet wired into `lib.rs`; expect an
"unused" warning at most, resolved in Step 4). If `cargo check` errors on the unreferenced module,
proceed to 4.1 first to wire `mod crawl;`, then return.

---

### Step 2: Classify files during the walk

#### 2.1 Add `classify(root, entry_path) -> Option<ContentFile>` to `src/crawl.rs`
**File:** `src/crawl.rs`
**Action:** add a private free function implementing the derivation rule from the Ground-truth
table.
- Signature: `fn classify(root: &Path, path: &Path) -> Option<ContentFile>`.
- Compute `rel = path.strip_prefix(root).ok()?`.
- Collect `rel.components()` as `&str` via `comp.as_os_str().to_str()` (return `None` if any
  component is non-UTF8 — caller turns that into a diagnostic, see 3.x is filename-only; non-UTF8
  paths are simply skipped here).
- First component must equal `"paths"` → else `return None` (skip).
- `path_id` = second component (if missing → `None`).
- If third component == `"pt-BR"`: `locale = PtBr`, tail starts at index 3; else `locale = En`,
  tail starts at index 2.
- Match the tail:
  - `["metadata.json"]` → `FileKind::PathMetadataJson`, `module_id = None`.
  - `["modules", file]` where `file` ends in `.json` → `LearnModuleJson`, `module_id = Some(stem)`.
  - `["modules", file]` where `file` ends in `.mdx` → `ModuleMdx`, `module_id = Some(stem)`.
  - anything else → `None` (skip).
- `stem` = filename without final extension (`Path::new(file).file_stem()`).

#### 2.2 Add the `crawl(root) -> (Corpus, Vec<Diagnostic>)` public entry to `src/crawl.rs`
**File:** `src/crawl.rs`
**Action:** add the public walk driver.
```rust
pub fn crawl(root: &Path) -> (Corpus, Vec<Diagnostic>) {
    let mut corpus = Corpus::default();
    let mut diags = Vec::new();
    for entry in walkdir::WalkDir::new(root).into_iter() {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => { diags.push(Diagnostic::error(root, "", format!("walk error: {e}"))); continue; }
        };
        if !entry.file_type().is_file() { continue; }
        if let Some(cf) = classify(root, entry.path()) {
            diags.extend(check_filename(&cf));   // added in Step 3
            corpus.files.push(cf);
        }
    }
    (corpus, diags)
}
```
- Do **not** panic on walk errors — surface as a `Diagnostic::error` and continue (spec step 4).
- Note: `check_filename` is referenced here but defined in Step 3; stub it as `fn check_filename(_: &ContentFile) -> Vec<Diagnostic> { Vec::new() }` until 3.1 to keep this compiling.

**Verify:** `cargo check` → compiles once `mod crawl;` is wired (Step 4.1). Order is flexible:
if you prefer, do 4.1 now so checks pass, then continue.

---

### Step 3: Port the filename-convention checks (`validateFileName`)

#### 3.1 Implement `check_filename(&ContentFile) -> Vec<Diagnostic>` in `src/crawl.rs`
**File:** `src/crawl.rs`
**Action:** replace the Step-2 stub with the real check. Operate on the **final filename**
(`cf.rel.file_name()`), reporting against `cf.rel` as the diagnostic file and `""` locator.
- **No spaces:** if the filename contains `' '` → `Diagnostic::error(cf.rel, "", "filename contains spaces: <name>")`.
- **Lowercase:** if `name != name.to_lowercase()` → `Diagnostic::error(cf.rel, "", "filename must be lowercase: <name>")`.
- **Module pattern:** for `LearnModuleJson` and `ModuleMdx` only, the filename must match
  `^\d{2}-[a-z0-9-]+\.(json|mdx)$`. `PathMetadataJson` is exempt (it is literally `metadata.json`).
  Implement the regex by hand (no `regex` crate — it is not a dependency and the spec expects no new
  deps): split on the first `-`, assert the prefix is exactly two ASCII digits, assert the remainder
  before the extension is all `[a-z0-9-]`, assert the extension matches the kind. On failure →
  `Diagnostic::error(cf.rel, "", "module filename must match NN-slug.(json|mdx): <name>")`.
- All three are **errors** (matches the TS validator's `validateFileName`, which fails the run).
- A file that fails a filename check is still pushed to the corpus (done in 2.2 — the diagnostic
  does not remove it), so downstream blocks still see it.

**Verify:** `cargo clippy -- -D warnings` → no warnings on `src/crawl.rs` (watch for
`needless_return`, `manual_map`; prefer iterator/`?` idioms).

---

### Step 4: Build the corpus and wire into `validate()`

#### 4.1 Register the module and re-export its public API in `src/lib.rs`
**File:** `src/lib.rs`
**Action:** add near the top of the file (after the module doc comment, before `use std::path::PathBuf;`):
```rust
mod crawl;
pub use crawl::{ContentFile, Corpus, FileKind, Locale, crawl};
```

#### 4.2 Rewrite the `validate()` body in `src/lib.rs`
**File:** `src/lib.rs`
**Action:** replace the Phase-0 stub body (lines ~84–90, the `pub fn validate(_root...) { Ok(Report::default()) }`).
```rust
/// Validate the content tree rooted at `root`.
///
/// Block B: crawl + classify + filename conventions. Struct/cross-file checks land in C–E,
/// each consuming the `Corpus` returned by [`crawl`].
pub fn validate(root: &std::path::Path) -> anyhow::Result<Report> {
    let (_corpus, diagnostics) = crawl::crawl(root);
    Ok(Report { diagnostics })
}
```
- Keep the public signature returning `anyhow::Result<Report>` so `src/main.rs` is untouched and
  exit-code plumbing stays intact.
- `_corpus` is unused *this block* (C–E consume it); the leading underscore silences clippy. Do not
  delete the binding — it documents that the corpus is built here.

**Verify:** `cargo build --release` → builds; `cargo run -- validate ../learn-ai/content/learn`
→ prints `validated …: N error(s), M warning(s)` and exits 0 (the live tree should have no
filename violations — confirm N reflects only any real filename issues, expected 0).

---

### Step 5: Tests against fixtures

#### 5.1 Create `tests/crawl.rs` with a fixture-tree helper
**File:** `tests/crawl.rs` (new)
**Action:** create the integration test file. Match the temp-dir style of `tests/smoke.rs`.
- Imports: `use std::fs;`, `use std::path::{Path, PathBuf};`, `use mev::{crawl, FileKind, Locale};`.
- Helper `fn write(root: &Path, rel: &str, body: &str)` that creates parent dirs and writes a file.
- Helper `fn fixture_root(name: &str) -> PathBuf` that returns `std::env::temp_dir().join(name)`,
  removing any prior copy first (mirror `smoke.rs`'s `remove_dir_all` pattern).

#### 5.2 Add the "good tree classifies correctly" test to `tests/crawl.rs`
**File:** `tests/crawl.rs`
**Action:** add `#[test] fn classifies_good_tree()`.
- Build a tree under the fixture root:
  - `paths/demo-path/metadata.json`
  - `paths/demo-path/modules/01-intro.json`, `paths/demo-path/modules/01-intro.mdx`
  - `paths/demo-path/pt-BR/metadata.json`
  - `paths/demo-path/pt-BR/modules/01-intro.mdx`
  - non-content noise that must be skipped: `schemas/module-schema.json`, `CLAUDE.md`,
    `shared/templates/x.json`
- Call `let (corpus, diags) = crawl(&root);`.
- Assert `diags.is_empty()` (all filenames valid).
- Assert `corpus.files.len() == 5` (only the five content files; noise skipped).
- Assert the `metadata.json` entry is `FileKind::PathMetadataJson`, `path_id == "demo-path"`,
  `module_id == None`, one with `Locale::En` and one with `Locale::PtBr`.
- Assert `01-intro.json` is `LearnModuleJson` with `module_id == Some("01-intro")`, `Locale::En`.
- Assert `corpus.get("demo-path", "01-intro", Locale::En, FileKind::ModuleMdx).is_some()`.
- Clean up the fixture root at the end.

#### 5.3 Add the "filename violations are diagnosed" test to `tests/crawl.rs`
**File:** `tests/crawl.rs`
**Action:** add `#[test] fn flags_bad_filenames()`.
- Build a tree with deliberately broken module filenames under `paths/demo-path/modules/`:
  - `01 intro.mdx` (space) — expect a "contains spaces" error.
  - `01-Intro.json` (uppercase) — expect a "must be lowercase" error.
  - `intro.mdx` (missing `NN-` prefix) — expect a "module filename must match" error.
  - one valid control: `02-valid.json`.
- Call `crawl(&root)`.
- Assert the diagnostics include exactly the three expected messages (assert on count == 3 and on a
  substring of each message), and that every diagnostic is `Severity::Error`.
- Assert the broken files are **still present** in `corpus.files` (filename failure does not drop
  them) — assert `corpus.files.len() == 4`.
- Clean up.

#### 5.4 Confirm the empty-tree smoke test still holds
**File:** `tests/smoke.rs` (no change expected)
**Action:** read-only — `empty_tree_produces_clean_report` must still pass now that `validate`
crawls. An empty dir yields no content files and an empty `Report`. If it fails (e.g. `validate`
errors on a missing `paths/` dir), fix `crawl` to treat an absent `paths/` as "no files", not an
error.

**Verify:** `cargo test` → all tests pass (`smoke.rs` 2 + `crawl.rs` 2 = 4 passing; 0 failures).

---

### Step 6: Validate

#### 6.1 Run the full gate suite
**File:** — (commands)
**Action:** run each, in order, and confirm all pass:
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

#### 6.2 Sanity-check against the live tree (optional, non-gating)
**File:** — (command)
**Action:** `cargo run -- validate ../learn-ai/content/learn` — confirm it enumerates the tree and
reports only real filename issues (expected: 0 errors on the current corpus at this block).

**Verify:** all four gate commands exit 0.

---

## Acceptance Criteria
- `Corpus` enumerates the live content tree, grouping files by `path_id` and `module_id` with the
  correct `FileKind` for each (`learn-module-json` / `path-metadata-json` / `module-mdx`).
- Non-content files (schemas, READMEs, dotfiles) are skipped without error.
- Every filename-convention violation (spaces, uppercase, missing `NN-` prefix / wrong pattern)
  surfaces as a `Diagnostic` with the correct severity and file locator.
- New fixture-driven tests cover good + deliberately-broken filenames and pass.
- All four harness gates are green; the existing `smoke.rs` tests still pass.

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes
- **Live layout (authoritative):** content modules live at `paths/<path-id>/modules/<NN-slug>.{json,mdx}`;
  the path's own metadata at `paths/<path-id>/metadata.json`; the pt-BR mirror nests the same shape
  under a `pt-BR/` segment. `schemas/`, `shared/`, and top-level `*.md` are not content — skip them.
- **No `regex` crate.** Cargo.toml has no `regex` dependency and the spec expects none added.
  Implement the `^\d{2}-[a-z0-9-]+\.(json|mdx)$` check by hand (char-class checks). This also keeps
  the `clippy`/`build` gates dependency-stable.
- **Determinism:** `walkdir` order is filesystem-dependent; tests assert on counts/lookups and (where
  ordering matters) sort, rather than on raw `files` order. Prefer `BTreeMap`/sorted accessors.
- **`validate()` contract preserved:** still returns `anyhow::Result<Report>`; `src/main.rs` needs no
  change. The `Corpus` is built and bound (`_corpus`) ready for Blocks C–E to consume.
- **CLAUDE.md rules in force:** every block ships tests (rule 1) — Step 5 satisfies it; all four
  gates must pass (Step 6); no new markdown files here, so no OKF-frontmatter obligation triggered.
