<!-- Archived 2026-06-29 — residue distilled into planning/knowledge.md -->

# Task Spec — Phase 2, Block G

**Status:** Done · **Last run:** 2026-06-26

## Goal
Add a parallel Brain crawl entry point — `MdFile { path, rel, stem }` + `crawl_brain(root)` that walks all `.md` under a root, pruning a name blocklist (`target/`, `node_modules/`, `.git/`) and any non-root directory containing its own `.git`.

## Context Pointers
- **Plan section:** `planning/master-plan.md` → Phase 2 → *Block G — Brain crawl*. The acceptance bar there is the source of truth for what the crawl must prune and find.
- **Pattern to mirror:** `src/learn_ai/crawl.rs` — the existing `walkdir`-based `crawl(root) -> (Corpus, Vec<Diagnostic>)`. Block G is a *parallel* crawl, not a retrofit: a new `src/brain/` module sibling to `src/learn_ai/`, with its own item type (`MdFile`) and walk. Do **not** touch the learn-ai crawl.
- **Module wiring:** `src/lib.rs` declares `mod learn_ai; mod shared; mod validator;` and re-exports the public surface via `pub use`. The new `brain` module is wired in the same way.
- **`walkdir` skip mechanics:** `WalkDir` entries expose `.depth()` (0 = the root itself) and `.path()`. Directory pruning is done with `IntoIter::filter_entry` or an explicit skip check — the nested-git rule must prune the *directory subtree*, so a directory-level decision (not a per-file one) is required. The `depth() > 0` guard exempts the brain root, which is itself a git repo.
- **Standing rules:** every block ships with tests (CLAUDE.md rule 1); all four harness gates stay green (fmt, clippy, test, build).
- **Scope boundary (from the plan):** Block G is crawl only. No OKF frontmatter parsing/validation (that is Block H), no `validate-brain` subcommand or `--json` (Block I). `crawl_brain` returns the `MdFile` list (plus any walk-error diagnostics, mirroring learn-ai's `(items, Vec<Diagnostic>)` shape) — it does not read file contents.

## Step-by-Step Tasks

### 1. Scaffold the `brain` module and `MdFile` type
- Create `src/brain/mod.rs` declaring `pub mod crawl;` (mirrors `src/learn_ai/mod.rs`).
- Create `src/brain/crawl.rs` with the `MdFile` struct: `path: PathBuf` (absolute path as walked), `rel: PathBuf` (path relative to the crawl root, for diagnostic locators/display), `stem: String` (file stem, e.g. `status` for `status.md`). Add a module-level doc comment describing the two-layer skip-list and the `depth() > 0` nested-git rule.
- Add a stub `pub fn crawl_brain(root: &Path) -> (Vec<MdFile>, Vec<Diagnostic>)` returning empty vecs, so the module compiles before the walk logic lands in Task 2.
- Wire the module into `src/lib.rs`: add `mod brain;` and `pub use brain::crawl::{MdFile, crawl_brain};`.
- Files: `src/brain/mod.rs` (new), `src/brain/crawl.rs` (new), `src/lib.rs` (modified — module decl + re-export).

### 2. Implement `crawl_brain` walk + two-layer skip-list
- Depends on Task 1. Owns `src/brain/crawl.rs`.
- Walk `root` with `walkdir::WalkDir`, using directory pruning so an excluded directory's entire subtree is skipped (e.g. `WalkDir::new(root).into_iter().filter_entry(|e| !should_skip_dir(e))`).
- Factor the directory-skip decision into a small testable helper (e.g. `fn is_blocklisted_name(name: &str) -> bool` for `target` / `node_modules` / `.git`, and the nested-git check). The **name blocklist** prunes any directory named `target`, `node_modules`, or `.git`. The **nested-git rule** prunes any directory at `depth() > 0` that contains its own `.git` entry (so every sub-project repo is excluded; the brain root at depth 0 is exempt).
- Collect every remaining `*.md` *file* (skip directories and non-`.md` files) into an `MdFile` with `path`, `rel` (via `strip_prefix(root)`), and `stem`. Surface walk/IO errors as `Diagnostic`s (mirror `learn_ai::crawl`’s error handling) rather than propagating `Err`.
- Files: `src/brain/crawl.rs` (modified).

### 3. Unit + integration tests for crawl + pruning
- Depends on Task 2. Adds a `#[cfg(test)] mod tests` in `src/brain/crawl.rs` and a new integration test file `tests/brain_crawl.rs` (sibling to `tests/crawl.rs`).
- **Unit tests** (in `crawl.rs`): cover the pruning helpers directly — `target`/`node_modules`/`.git` names are blocklisted, ordinary names are not; the nested-git predicate prunes a `depth() > 0` dir with a `.git` child and exempts the root.
- **Integration tests** (`tests/brain_crawl.rs`): build a temp-dir fixture (use the same temp-dir harness style as `tests/crawl.rs`) and assert: (a) a root-level `.md` is found; (b) a `.md` inside a `target/` dir is pruned; (c) a `.md` inside a nested-git sub-dir (a non-root dir containing a `.git` marker) is pruned; (d) a non-`.md` file (e.g. `notes.txt`) is skipped; (e) `MdFile.rel` and `MdFile.stem` carry the expected values for a found file.
- Files: `src/brain/crawl.rs` (modified — test module), `tests/brain_crawl.rs` (new).

### 4. Validate
- Run the Validation Commands listed below and confirm all pass.

## Acceptance Criteria
- `MdFile { path, rel, stem }` and `pub fn crawl_brain(root: &Path) -> (Vec<MdFile>, Vec<Diagnostic>)` exist and are re-exported from `src/lib.rs`.
- `crawl_brain` returns every root-level and nested `.md` file *except* those under a `target/`, `node_modules/`, or `.git/` directory, or under any non-root directory that contains its own `.git`.
- A `.md` file inside a nested-git sub-directory is pruned; a root-level `.md` is still found (the brain root's own `.git` does not prune the root).
- Non-`.md` files are skipped (never returned as `MdFile`s).
- New unit tests prove the blocklist and nested-git pruning helpers; new integration tests prove the end-to-end crawl behaviour against a temp-dir fixture.
- The existing learn-ai crawl and its tests are unchanged and still pass.
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
