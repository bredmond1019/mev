# Worklog — 2.F-content-validator-trait

## Task 1 — PASSED (1 attempt)
What: Created src/shared.rs with extract_frontmatter, is_kebab_case, and non_empty helpers (+ tests) moved from src/meta.rs; added mod shared to lib.rs; updated meta.rs to import from crate::shared
Decisions: Added a non_empty_helper unit test in shared.rs since the helper had no dedicated test in meta.rs (only implicit coverage), satisfying the 'every block ships with tests' standing rule
Validated: gating checks (fast tripwire)
