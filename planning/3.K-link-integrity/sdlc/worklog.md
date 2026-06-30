# Worklog — 3.K-link-integrity

## Task 1 — PASSED (1 attempt)
What: Add src/brain/links.rs with LinkKind/LinkRef model and extract_links() extractor; register module in mod.rs
Decisions: extract_links uses a single-pass byte-scan state machine rather than a regex crate to keep dependencies minimal and match the existing codebase pattern; bare file:// URIs in prose are extracted in addition to those inside markdown link targets, consistent with the spec's 'file:// / file:/// URIs (in markdown link targets or bare in prose)' wording; wikilink anchor stripping is implemented per spec even though it is unusual, since the spec says 'target = path/slug portion with any #anchor suffix stripped'
Validated: gating checks (fast tripwire)

## Task 2 — PASSED (1 attempt)
What: Added check_links (resolves Markdown/FileUri/WikiLink refs with E_LINK_* diagnostics) and collect_doc_ids (D5-seam bare doc_id extractor) to src/brain/links.rs, with 9 new unit tests covering all required scenarios.
Decisions: _root parameter kept in check_links signature for symmetry with sibling check_* functions (not used in this pass — relative markdown links resolve from entry.path); file:// stripping uses strip_prefix("file://") so file:///abs/path → /abs/path correctly; brain_only_config helper removed from tests — not needed since all tests construct CorpusEntry directly
Validated: gating checks (fast tripwire)

## Task 3 — PASSED (1 attempt)
What: Added read_moves_pending and check_moved_references to src/brain/links.rs; missing .brain-moves-pending produces no diagnostics, stale markdown/file:// refs to moved paths emit E_LINK_MOVED_REFERENCE
Decisions: normalize_path uses lexical component walking (no canonicalize) so it works on paths that may no longer exist on disk; WikiLink targets are slug-based and excluded from moved-reference scanning — only Markdown and FileUri links are path-resolved against moved_paths
Validated: gating checks (fast tripwire)
