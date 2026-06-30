//! Link integrity model, extractor, and checker (Phase 3, Block K).
//!
//! This module provides:
//! - A serializable link model ([`LinkKind`] / [`LinkRef`]) mirroring the graph
//!   model's D4 `serde::Serialize` derives for forward-compat.
//! - [`extract_links`] — scans a Markdown/MDX file body for inline markdown links,
//!   `file://` URIs, and `[[wikilink]]` references, skipping external URLs and
//!   pure in-page anchors.
//!
//! # Diagnostic locator codes (Phase 3 Block K)
//! - `E_LINK_DEAD_MARKDOWN` — a relative markdown `[text](path)` resolves to no file on disk.
//! - `E_LINK_DEAD_FILE_URI` — a `file://` URI resolves to no file on disk.
//! - `E_LINK_DANGLING_WIKILINK` — a `[[wikilink]]` slug is not a known `doc_id`.
//! - `E_LINK_MOVED_REFERENCE` — a reference still points at a path listed in `.brain-moves-pending`.
//!
//! - [`check_links`] — resolves each extracted link against the filesystem or the
//!   `doc_ids` set, emitting `E_LINK_*` diagnostics for broken references.
//! - [`collect_doc_ids`] — collects the set of all authored bare `doc_id`s from a
//!   corpus, reusing the D5 `read_doc_metadata` seam.
//!
//! Tasks 2–3 (`check_links`, `check_moved_references`) extend this module.

use std::collections::HashSet;
use std::path::Path;

use serde::Serialize;

use crate::Diagnostic;
use crate::brain::crawl::Corpus;
use crate::brain::graph::read_doc_metadata;

// ---------------------------------------------------------------------------
// Link model (D4 serializable)
// ---------------------------------------------------------------------------

/// The kind of a local reference found in a Markdown/MDX file.
///
/// External URLs (`http(s)://`, `mailto:`, `tel:`, protocol-relative `//`) and
/// pure in-page anchors (`#section`) are excluded before a [`LinkRef`] is created.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    /// Inline markdown link: `[text](target)`.
    Markdown,
    /// `file://` or `file:///` URI.
    FileUri,
    /// Double-bracket wikilink: `[[slug]]`.
    WikiLink,
}

/// A local reference found in a Markdown/MDX file body.
///
/// `raw` is the as-authored reference string (the full target including any anchor
/// suffix). `target` is the path / slug portion with any `#anchor` suffix stripped
/// (anchors are out of scope for Block K).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LinkRef {
    /// The kind of link.
    pub kind: LinkKind,
    /// The as-authored reference string (e.g. `"../foo.md#section"` or `"[[my-doc]]"`).
    pub raw: String,
    /// The path / slug portion with any `#anchor` stripped.
    pub target: String,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Strip a `#anchor` suffix from a path/target string.
///
/// Only strips if the `#` is preceded by at least one character — a bare `#section`
/// (pure in-page anchor) has an empty prefix and is handled by the caller before
/// calling this.
fn strip_anchor(s: &str) -> &str {
    match s.find('#') {
        Some(pos) if pos > 0 => &s[..pos],
        _ => s,
    }
}

/// Return `true` for targets that are external or that should be skipped entirely.
///
/// Skipped: `http://`, `https://`, `mailto:`, `tel:`, protocol-relative `//`, and
/// pure in-page anchors (`#section`).
fn is_external_or_skipped(target: &str) -> bool {
    if target.starts_with("http://")
        || target.starts_with("https://")
        || target.starts_with("mailto:")
        || target.starts_with("tel:")
        || target.starts_with("//")
        || target.starts_with('#')
    {
        return true;
    }
    false
}

// ---------------------------------------------------------------------------
// extract_links
// ---------------------------------------------------------------------------

/// Extract all local link references from `contents` (a Markdown/MDX file body).
///
/// The function scans for three reference kinds:
/// - Inline markdown links `[text](target)` → [`LinkKind::Markdown`].
/// - `file://` / `file:///` URIs → [`LinkKind::FileUri`].
/// - `[[wikilink]]` → [`LinkKind::WikiLink`].
///
/// External / non-local references (`http(s)://`, `mailto:`, `tel:`, `//`, `#…`)
/// are silently skipped — they produce no [`LinkRef`].
///
/// The `target` field always has any `#anchor` suffix stripped; the as-authored
/// text is preserved in `raw`.
pub fn extract_links(contents: &str) -> Vec<LinkRef> {
    let mut links: Vec<LinkRef> = Vec::new();

    // Scan character-by-character with a simple state machine approach.
    // We look for three patterns:
    //   1. [text](target)  — markdown inline link
    //   2. [[slug]]        — wikilink
    //   3. file:// or file:///  — file URI (as a standalone URI in text or inside a markdown link target)
    //
    // Strategy: scan with byte-level find for pattern anchors.

    let bytes = contents.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        // Try wikilink first: [[...]]
        if i + 3 < len && bytes[i] == b'[' && bytes[i + 1] == b'[' {
            // Look for closing ]]
            if let Some(close) = find_close(contents, i + 2, "]]") {
                let inner = &contents[i + 2..close];
                // Exclude if starts with [ (malformed) or is empty
                if !inner.is_empty() && !inner.contains(']') {
                    let raw = format!("[[{inner}]]");
                    let target = strip_anchor(inner).to_string();
                    links.push(LinkRef {
                        kind: LinkKind::WikiLink,
                        raw,
                        target,
                    });
                    i = close + 2;
                    continue;
                }
            }
        }

        // Try markdown inline link: [text](target)
        // We look for '](' which anchors the pattern
        if bytes[i] == b'[' && !matches!(bytes.get(i + 1), Some(b'[')) {
            // Find the text part: [text]
            if let Some(bracket_close) = find_close(contents, i + 1, "]") {
                let after_bracket = bracket_close + 1;
                if after_bracket < len && bytes[after_bracket] == b'(' {
                    // Find the closing paren
                    if let Some(paren_close) = find_close(contents, after_bracket + 1, ")") {
                        let raw_target = &contents[after_bracket + 1..paren_close];
                        // Strip any title: `path "title"` → take the path part
                        let path_part = raw_target.split_whitespace().next().unwrap_or("");

                        if !is_external_or_skipped(path_part) && !path_part.is_empty() {
                            // Determine kind: file:// URI or relative markdown
                            let kind = if path_part.starts_with("file://") {
                                LinkKind::FileUri
                            } else {
                                LinkKind::Markdown
                            };
                            let target = strip_anchor(path_part).to_string();
                            links.push(LinkRef {
                                kind,
                                raw: path_part.to_string(),
                                target,
                            });
                        }
                        i = paren_close + 1;
                        continue;
                    }
                }
            }
        }

        // Try bare file:// URI in prose (not inside a markdown link target)
        if contents[i..].starts_with("file://") {
            // Extract the URI up to whitespace or end of line or common delimiters
            let rest = &contents[i..];
            let end = rest
                .find(|c: char| c.is_whitespace() || matches!(c, ')' | '>' | '"' | '\''))
                .unwrap_or(rest.len());
            let raw = &rest[..end];
            let target = strip_anchor(raw).to_string();
            links.push(LinkRef {
                kind: LinkKind::FileUri,
                raw: raw.to_string(),
                target,
            });
            i += end;
            continue;
        }

        i += 1;
    }

    links
}

/// Find the position of `needle` starting from `start` in `s`, returning the byte
/// offset of the start of the needle (not its end) if found.
fn find_close(s: &str, start: usize, needle: &str) -> Option<usize> {
    s[start..].find(needle).map(|pos| start + pos)
}

// ---------------------------------------------------------------------------
// collect_doc_ids — D5 single-seam helper
// ---------------------------------------------------------------------------

/// Collect the set of all authored bare `doc_id`s from the corpus.
///
/// Reuses [`read_doc_metadata`] (the D5 single frontmatter-parsing seam) rather
/// than re-parsing frontmatter independently.  Returns bare `doc_id` strings (no
/// `scope:` prefix) so wikilink targets (which are scope-agnostic slugs) can be
/// looked up directly.
pub fn collect_doc_ids(corpus: &Corpus) -> HashSet<String> {
    corpus
        .entries
        .iter()
        .filter_map(|entry| read_doc_metadata(entry).doc_id)
        .collect()
}

// ---------------------------------------------------------------------------
// check_links — resolution pass
// ---------------------------------------------------------------------------

/// Strip the `file://` scheme from a URI, returning the raw path portion.
///
/// `file:///absolute/path` → `/absolute/path`
/// `file://host/path` → `host/path`
fn file_uri_to_path(uri: &str) -> &str {
    uri.strip_prefix("file://").unwrap_or(uri)
}

/// Resolve and check all local link references in every [`CorpusEntry`].
///
/// For each entry the function reads its contents (graceful degrade on I/O error —
/// mirrors [`read_doc_metadata`]), extracts links via [`extract_links`], and
/// resolves each [`LinkRef`]:
///
/// - **[`LinkKind::Markdown`]** — resolves the target relative to the referring
///   file's directory.  A missing file → `E_LINK_DEAD_MARKDOWN` (error).
/// - **[`LinkKind::FileUri`]** — strips the `file://`/`file:///` scheme and checks
///   the resulting path on disk.  Missing → `E_LINK_DEAD_FILE_URI` (error).
/// - **[`LinkKind::WikiLink`]** — checks the target slug against `doc_ids` (the
///   set of all authored bare `doc_id`s, scope-agnostic).  Unknown slug →
///   `E_LINK_DANGLING_WIKILINK` (error).
///
/// `root` is accepted for signature symmetry with sibling `check_*` functions but
/// is not used in this pass (relative markdown links resolve from `entry.path`).
pub fn check_links(corpus: &Corpus, _root: &Path, doc_ids: &HashSet<String>) -> Vec<Diagnostic> {
    let mut diags: Vec<Diagnostic> = Vec::new();

    for entry in &corpus.entries {
        let contents = match std::fs::read_to_string(&entry.path) {
            Ok(c) => c,
            // Graceful degrade — skip unreadable files (mirrors read_doc_metadata).
            Err(_) => continue,
        };

        let links = extract_links(&contents);

        for link in &links {
            match link.kind {
                LinkKind::Markdown => {
                    // Resolve relative to the referring file's directory.
                    let base = entry.path.parent().unwrap_or_else(|| Path::new("."));
                    let resolved = base.join(&link.target);
                    if !resolved.exists() {
                        diags.push(Diagnostic::error(
                            &entry.rel,
                            "E_LINK_DEAD_MARKDOWN",
                            format!(
                                "dead markdown link: '{}' does not exist (resolved: '{}')",
                                link.raw,
                                resolved.display()
                            ),
                        ));
                    }
                }
                LinkKind::FileUri => {
                    let path_str = file_uri_to_path(&link.target);
                    let path = Path::new(path_str);
                    if !path.exists() {
                        diags.push(Diagnostic::error(
                            &entry.rel,
                            "E_LINK_DEAD_FILE_URI",
                            format!(
                                "dead file URI: '{}' does not exist (path: '{}')",
                                link.raw, path_str
                            ),
                        ));
                    }
                }
                LinkKind::WikiLink => {
                    if !doc_ids.contains(&link.target) {
                        diags.push(Diagnostic::error(
                            &entry.rel,
                            "E_LINK_DANGLING_WIKILINK",
                            format!(
                                "dangling wikilink: '[[{}]]' is not a known doc_id",
                                link.target
                            ),
                        ));
                    }
                }
            }
        }
    }

    diags
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- markdown inline link ---

    #[test]
    fn relative_markdown_link_extracted() {
        let links = extract_links("See [the guide](../docs/guide.md) for details.");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].kind, LinkKind::Markdown);
        assert_eq!(links[0].raw, "../docs/guide.md");
        assert_eq!(links[0].target, "../docs/guide.md");
    }

    #[test]
    fn markdown_link_with_anchor_strips_anchor() {
        let links = extract_links("[see this](path/to/file.md#section)");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].kind, LinkKind::Markdown);
        assert_eq!(links[0].raw, "path/to/file.md#section");
        assert_eq!(links[0].target, "path/to/file.md");
    }

    // --- file:// link ---

    #[test]
    fn file_uri_in_markdown_link_extracted() {
        let links = extract_links("[open](file:///home/user/doc.md)");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].kind, LinkKind::FileUri);
        assert!(links[0].target.starts_with("file:///"));
    }

    #[test]
    fn bare_file_uri_in_prose_extracted() {
        let links = extract_links("See file:///home/user/notes.md here.");
        assert_eq!(links.len(), 1, "expected 1 link, got: {links:?}");
        assert_eq!(links[0].kind, LinkKind::FileUri);
        assert_eq!(links[0].raw, "file:///home/user/notes.md");
        assert_eq!(links[0].target, "file:///home/user/notes.md");
    }

    // --- wikilink ---

    #[test]
    fn wikilink_extracted() {
        let links = extract_links("See [[my-doc]] for context.");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].kind, LinkKind::WikiLink);
        assert_eq!(links[0].raw, "[[my-doc]]");
        assert_eq!(links[0].target, "my-doc");
    }

    #[test]
    fn wikilink_with_anchor_strips_anchor() {
        // Anchors on wikilinks are unusual but we strip them per spec.
        let links = extract_links("See [[my-doc#section]] here.");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].kind, LinkKind::WikiLink);
        assert_eq!(links[0].raw, "[[my-doc#section]]");
        assert_eq!(links[0].target, "my-doc");
    }

    // --- skipped external / in-page anchor links ---

    #[test]
    fn http_link_skipped() {
        let links = extract_links("[external](https://example.com/page)");
        assert!(links.is_empty(), "https link should be skipped: {links:?}");
    }

    #[test]
    fn http_no_s_link_skipped() {
        let links = extract_links("[external](http://example.com/page)");
        assert!(links.is_empty(), "http link should be skipped: {links:?}");
    }

    #[test]
    fn mailto_link_skipped() {
        let links = extract_links("[email](mailto:foo@example.com)");
        assert!(links.is_empty(), "mailto link should be skipped: {links:?}");
    }

    #[test]
    fn tel_link_skipped() {
        let links = extract_links("[call](tel:+15555555555)");
        assert!(links.is_empty(), "tel link should be skipped: {links:?}");
    }

    #[test]
    fn protocol_relative_link_skipped() {
        let links = extract_links("[resource](//cdn.example.com/script.js)");
        assert!(
            links.is_empty(),
            "protocol-relative link should be skipped: {links:?}"
        );
    }

    #[test]
    fn pure_anchor_link_skipped() {
        let links = extract_links("[section](#introduction)");
        assert!(
            links.is_empty(),
            "pure in-page anchor should be skipped: {links:?}"
        );
    }

    // --- multiple links in one document ---

    #[test]
    fn multiple_link_kinds_in_one_doc() {
        let content = concat!(
            "See [guide](docs/guide.md) and [[overview]] and ",
            "[skip](https://example.com) and [anchor](#top).",
        );
        let links = extract_links(content);
        // Only the relative markdown link and wikilink should be present.
        assert_eq!(links.len(), 2, "expected 2 local links, got: {links:?}");
        assert_eq!(links[0].kind, LinkKind::Markdown);
        assert_eq!(links[1].kind, LinkKind::WikiLink);
    }

    // =========================================================================
    // Task 2 tests: check_links + collect_doc_ids
    // =========================================================================

    use std::collections::HashSet;
    use std::path::Path;

    use crate::brain::crawl::{Corpus, CorpusEntry};

    /// Write `content` to a temp file and return a minimal [`CorpusEntry`] for it.
    fn write_corpus_entry(dir: &std::path::Path, rel_str: &str, content: &str) -> CorpusEntry {
        let full = dir.join(rel_str);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&full, content.as_bytes()).unwrap();
        CorpusEntry {
            path: full,
            rel: std::path::PathBuf::from(rel_str),
            stem: std::path::Path::new(rel_str)
                .file_stem()
                .unwrap()
                .to_str()
                .unwrap()
                .to_string(),
            scope: "brain".to_string(),
        }
    }

    // --- dead relative markdown link is flagged ---

    #[test]
    fn dead_markdown_link_is_flagged() {
        let dir = std::env::temp_dir().join("mev-links-dead-md");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("docs")).unwrap();

        let entry = write_corpus_entry(
            &dir,
            "docs/page.md",
            "See [missing](../planning/gone.md) here.",
        );
        let corpus = Corpus {
            entries: vec![entry],
        };
        let doc_ids = HashSet::new();
        let diags = check_links(&corpus, &dir, &doc_ids);

        assert_eq!(diags.len(), 1, "expected 1 diagnostic, got: {diags:?}");
        assert_eq!(diags[0].locator, "E_LINK_DEAD_MARKDOWN");
        assert!(diags[0].message.contains("gone.md"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- live relative markdown link does NOT produce a diagnostic ---

    #[test]
    fn live_markdown_link_passes() {
        let dir = std::env::temp_dir().join("mev-links-live-md");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("docs")).unwrap();
        std::fs::create_dir_all(dir.join("planning")).unwrap();
        std::fs::write(dir.join("planning/status.md"), b"").unwrap();

        let entry = write_corpus_entry(
            &dir,
            "docs/page.md",
            "See [status](../planning/status.md) here.",
        );
        let corpus = Corpus {
            entries: vec![entry],
        };
        let doc_ids = HashSet::new();
        let diags = check_links(&corpus, &dir, &doc_ids);

        assert!(diags.is_empty(), "expected no diagnostics, got: {diags:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- dead file:// URI is flagged ---

    #[test]
    fn dead_file_uri_is_flagged() {
        let dir = std::env::temp_dir().join("mev-links-dead-file-uri");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("docs")).unwrap();

        // We reference an absolute path that definitely does not exist.
        let entry = write_corpus_entry(
            &dir,
            "docs/page.md",
            "Open [doc](file:///tmp/mev-nonexistent-12345/file.md) now.",
        );
        let corpus = Corpus {
            entries: vec![entry],
        };
        let doc_ids = HashSet::new();
        let diags = check_links(&corpus, &dir, &doc_ids);

        assert_eq!(diags.len(), 1, "expected 1 diagnostic, got: {diags:?}");
        assert_eq!(diags[0].locator, "E_LINK_DEAD_FILE_URI");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- live file:// URI does NOT produce a diagnostic ---

    #[test]
    fn live_file_uri_passes() {
        let dir = std::env::temp_dir().join("mev-links-live-file-uri");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("docs")).unwrap();

        // Create the file first, then reference it.
        let target_path = dir.join("planning/status.md");
        std::fs::create_dir_all(target_path.parent().unwrap()).unwrap();
        std::fs::write(&target_path, b"").unwrap();

        let uri = format!("file://{}", target_path.display());
        let content = format!("See [status]({uri}) here.");
        let entry = write_corpus_entry(&dir, "docs/page.md", &content);

        let corpus = Corpus {
            entries: vec![entry],
        };
        let doc_ids = HashSet::new();
        let diags = check_links(&corpus, &dir, &doc_ids);

        assert!(diags.is_empty(), "expected no diagnostics, got: {diags:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- dangling wikilink is flagged ---

    #[test]
    fn dangling_wikilink_is_flagged() {
        let dir = std::env::temp_dir().join("mev-links-dangling-wiki");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("docs")).unwrap();

        let entry = write_corpus_entry(&dir, "docs/page.md", "See [[unknown-slug]] here.");
        let corpus = Corpus {
            entries: vec![entry],
        };
        // doc_ids does NOT contain "unknown-slug"
        let doc_ids: HashSet<String> = vec!["other-slug".to_string()].into_iter().collect();
        let diags = check_links(&corpus, &dir, &doc_ids);

        assert_eq!(diags.len(), 1, "expected 1 diagnostic, got: {diags:?}");
        assert_eq!(diags[0].locator, "E_LINK_DANGLING_WIKILINK");
        assert!(diags[0].message.contains("unknown-slug"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- wikilink to a real doc_id does NOT produce a diagnostic ---

    #[test]
    fn known_wikilink_passes() {
        let dir = std::env::temp_dir().join("mev-links-known-wiki");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("docs")).unwrap();

        let entry = write_corpus_entry(&dir, "docs/page.md", "See [[my-doc]] here.");
        let corpus = Corpus {
            entries: vec![entry],
        };
        let doc_ids: HashSet<String> = vec!["my-doc".to_string()].into_iter().collect();
        let diags = check_links(&corpus, &dir, &doc_ids);

        assert!(diags.is_empty(), "expected no diagnostics, got: {diags:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- external and anchor links are never flagged ---

    #[test]
    fn external_and_anchor_links_never_flagged() {
        let dir = std::env::temp_dir().join("mev-links-ext-anchor");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("docs")).unwrap();

        let content = concat!(
            "See [external](https://example.com) and ",
            "[mailto](mailto:foo@example.com) and ",
            "[anchor](#section) and ",
            "[proto-rel](//cdn.example.com/script.js)."
        );
        let entry = write_corpus_entry(&dir, "docs/page.md", content);
        let corpus = Corpus {
            entries: vec![entry],
        };
        let doc_ids = HashSet::new();
        let diags = check_links(&corpus, &dir, &doc_ids);

        assert!(
            diags.is_empty(),
            "expected no diagnostics for external/anchor links, got: {diags:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- collect_doc_ids extracts bare doc_ids from corpus entries ---

    #[test]
    fn collect_doc_ids_returns_authored_ids() {
        let dir = std::env::temp_dir().join("mev-links-collect-ids");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("docs")).unwrap();

        let content_a =
            "---\ntype: Reference\ntitle: A\ndescription: A doc.\ndoc_id: alpha\n---\nBody.";
        let content_b =
            "---\ntype: Reference\ntitle: B\ndescription: B doc.\ndoc_id: beta\n---\nBody.";
        let content_no_id = "---\ntype: Reference\ntitle: C\ndescription: No id.\n---\nBody.";

        let entry_a = write_corpus_entry(&dir, "docs/a.md", content_a);
        let entry_b = write_corpus_entry(&dir, "docs/b.md", content_b);
        let entry_c = write_corpus_entry(&dir, "docs/c.md", content_no_id);

        let corpus = Corpus {
            entries: vec![entry_a, entry_b, entry_c],
        };

        let ids = collect_doc_ids(&corpus);
        assert!(ids.contains("alpha"), "expected 'alpha' in ids: {ids:?}");
        assert!(ids.contains("beta"), "expected 'beta' in ids: {ids:?}");
        assert_eq!(
            ids.len(),
            2,
            "expected exactly 2 ids (no id for c): {ids:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- unreadable corpus entry is gracefully skipped (no panic) ---

    #[test]
    fn unreadable_entry_is_skipped_gracefully() {
        // Construct a CorpusEntry pointing at a path that does not exist.
        let entry = CorpusEntry {
            path: std::path::PathBuf::from("/tmp/mev-nonexistent-corpus-entry/file.md"),
            rel: std::path::PathBuf::from("docs/file.md"),
            stem: "file".to_string(),
            scope: "brain".to_string(),
        };
        let corpus = Corpus {
            entries: vec![entry],
        };
        let doc_ids = HashSet::new();
        // Should not panic; should return no diagnostics (file skipped).
        let diags = check_links(&corpus, Path::new("/tmp"), &doc_ids);
        assert!(
            diags.is_empty(),
            "expected no diagnostics for unreadable entry, got: {diags:?}"
        );
    }
}
