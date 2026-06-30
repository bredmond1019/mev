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
//! # Scope rule
//! Task 1 owns the model + extractor only. Resolution checks (`check_links`,
//! `check_moved_references`) are added in Tasks 2–3.

use serde::Serialize;

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
}
