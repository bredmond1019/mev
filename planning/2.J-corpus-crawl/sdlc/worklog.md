# Worklog — 2.J-corpus-crawl

## Task 1 — PASSED (1 attempt)
What: Add src/brain/scope.rs: registry-driven scope resolver (scope_units, scope_for, owning_unit) with 9 unit tests; register pub mod scope in mod.rs
Decisions: config.rs and tests/brain_config.rs changes are formatting-only (cargo fmt); staged alongside scope.rs since they were modified by the formatter during the gate run; Used Path::strip_prefix for prefix matching instead of string comparison to prevent false matches like core/mev-extra matching core/mev; The root unit (repo_path = '.') is excluded from prefix comparison and used only as the fallback
Validated: gating checks (fast tripwire)

## Task 2 — PASSED (1 attempt)
What: Add owned serializable Corpus/CorpusEntry types and crawl_corpus() to src/brain/crawl.rs; remove CLAUDE.md from is_blocklisted_file so it joins the corpus as a root leaf
Decisions: Named the brain corpus struct Corpus (in brain::crawl module) rather than BrainCorpus — no lib.rs re-export added, so no collision with the existing learn_ai::crawl::Corpus export; rel_to_unit_root returns Option<&Path> (None on prefix mismatch) rather than panicking — unexpected mismatches surface as a diagnostic and are skipped; Kept crawl_brain unchanged (still uses nested-git pruning and is_blocklisted_file) since it is still called by BrainValidator::crawl and has existing integration tests that rely on its current semantics
Validated: gating checks (fast tripwire)

## Task 3 — PASSED (1 attempt)
What: Wired crawl_corpus into BrainValidator::crawl and added OKF exemption for root instruction files (README.md/CLAUDE.md) without frontmatter
Decisions: Kept Item=MdFile on ContentValidator and mapped CorpusEntry->MdFile in crawl() rather than changing the trait Item type — avoids touching the trait definition and validate_item signature; Integration tests in brain_okf.rs and brain_validate.rs updated to place files under planning/ so they are corpus members (root-level stray .md files are no longer corpus members under the new crawl rules); Renamed brain_validator_prunes_nested_git_repos to _prunes_non_corpus_files since corpus membership exclusion now serves the same pruning purpose
Validated: gating checks (fast tripwire)

## Task 4 — PASSED (1 attempt)
What: Add tests/brain_corpus.rs: 13 integration tests for the multi-root corpus crawl over a 3-unit fixture tree (brain/core/mev), covering all positive corpus members, all spec-listed negative cases, scope correctness, and serde_json serialization.
Decisions: Used in-process BrainConfig construction (not a brain.toml file) for the fixture config, consistent with how other integration tests in this codebase are written — keeps tests self-contained and fast.; Included all 13 bloat skip_dirs from the spec (target, node_modules, .git, .claude, .repo-backups, .agent, .agents, archive, archived, trees, sdlc, venv, .venv) in the fixture config to match the canonical brain.toml list.
Validated: gating checks (fast tripwire)
