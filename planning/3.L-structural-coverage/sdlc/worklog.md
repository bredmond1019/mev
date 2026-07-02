# Worklog — 3.L-structural-coverage

## Task 1 — PASSED (1 attempt)
What: Added src/brain/structure.rs implementing check_structure(corpus, root) — bidirectional index.md/directory coverage, emitting E_STRUCT_ORPHAN_FILE and E_STRUCT_DANGLING_ROW diagnostics, registered via pub mod structure in src/brain/mod.rs, with 7 unit tests.
Decisions: Identified index.md entries by checking entry.path.file_name() == Some("index.md") rather than relying on stem alone, to be precise and match existing crawl conventions.; Reused links::extract_links + a local lexical normalize() helper (mirroring links.rs::normalize_path) rather than adding a shared cross-module helper, since the task spec called for a small private helper local to structure.rs.; FileUri targets are resolved as absolute paths (stripping the file:// scheme) rather than joined to the index.md's directory, consistent with check_links' existing FileUri handling.
Validated: gating checks (fast tripwire)
