# Worklog — 3.K-link-integrity

## Task 1 — PASSED (1 attempt)
What: Add src/brain/links.rs with LinkKind/LinkRef model and extract_links() extractor; register module in mod.rs
Decisions: extract_links uses a single-pass byte-scan state machine rather than a regex crate to keep dependencies minimal and match the existing codebase pattern; bare file:// URIs in prose are extracted in addition to those inside markdown link targets, consistent with the spec's 'file:// / file:/// URIs (in markdown link targets or bare in prose)' wording; wikilink anchor stripping is implemented per spec even though it is unusual, since the spec says 'target = path/slug portion with any #anchor suffix stripped'
Validated: gating checks (fast tripwire)
