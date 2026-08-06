//! Shared content-lint passes (Phase 12, Block A, Task 1).
//!
//! Two pure `(path, source) -> Vec<Diagnostic>` helpers that both `BlogValidator` and the
//! opt-in module lint pass on `LearnAiValidator` consume:
//!
//! - [`lint_code_blocks`] — scans fenced code blocks for a missing language tag
//!   (`W_LINT_UNTAGGED_CODE_BLOCK`).
//! - [`lint_local_links`] — extracts markdown links and image references, skips absolute
//!   URLs and in-page anchors, and checks that every remaining relative target exists on
//!   disk (`E_LINT_DEAD_LOCAL_LINK` / `E_LINT_DEAD_ASSET`).
//!
//! Both are pure over the given source text — no filesystem walk, one `Path::exists` call
//! per resolved link target — so they are trivially testable and reusable by any consumer.

use std::path::Path;

use crate::Diagnostic;

/// Scan fenced code blocks (``` and ~~~, honouring longer fences) and emit a
/// `W_LINT_UNTAGGED_CODE_BLOCK` warning for every opening fence with no language tag.
///
/// A fence character run of length >= 3 opens a block; the matching close is a run of the
/// same fence character at least as long, appearing on its own (only whitespace besides it)
/// once a block is open. Fence markers encountered while a block is already open (e.g. a
/// ```` ``` ```` fence nested inside a ```` ~~~ ```` block, or vice versa) do not open a
/// second block and are not double-counted.
pub fn lint_code_blocks(rel: &Path, source: &str) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    let mut open_fence: Option<(char, usize)> = None; // (fence char, run length)

    for (idx, line) in source.lines().enumerate() {
        let line_no = idx + 1;
        let trimmed = line.trim_start();
        let fence_char = trimmed.chars().next();
        let Some(fence_char) = fence_char else {
            continue;
        };
        if fence_char != '`' && fence_char != '~' {
            continue;
        }
        let run_len = trimmed.chars().take_while(|&c| c == fence_char).count();
        if run_len < 3 {
            continue;
        }
        let after_fence = &trimmed[run_len..];

        match open_fence {
            None => {
                // Opening fence: language tag is whatever follows the fence marker, trimmed.
                if after_fence.trim().is_empty() {
                    diags.push(Diagnostic::warning(
                        rel.to_path_buf(),
                        "W_LINT_UNTAGGED_CODE_BLOCK",
                        format!("fenced code block with no language tag at line {line_no}"),
                    ));
                }
                open_fence = Some((fence_char, run_len));
            }
            Some((open_char, open_len)) => {
                // Only a same-character fence of at least the opening run length, with
                // nothing but whitespace after it, closes the currently open block.
                if fence_char == open_char && run_len >= open_len && after_fence.trim().is_empty() {
                    open_fence = None;
                }
                // Any other fence marker while a block is open is inert content inside the
                // block (e.g. a shorter or differently-charactered fence) — ignore it.
            }
        }
    }

    diags
}

/// Extract markdown links (`[text](target)`) and images (`![alt](target)`) from `source` and
/// emit a diagnostic for every relative target that does not exist on disk.
///
/// Absolute URLs (`http://`, `https://`, `mailto:`) and protocol-relative (`//`) targets are
/// skipped — no network calls, ever. Pure in-page anchors (`#...`) are skipped. Everything
/// else is resolved relative to `file`'s parent directory, after stripping any trailing
/// `#anchor` or `?query` suffix, and checked for existence: `E_LINT_DEAD_ASSET` for an image
/// reference, `E_LINT_DEAD_LOCAL_LINK` for a link.
pub fn lint_local_links(file: &Path, rel: &Path, source: &str) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    let base = file.parent().unwrap_or_else(|| Path::new("."));

    let bytes = source.as_bytes();
    let mut i = 0usize;
    // Track 1-indexed line number by counting newlines consumed so far.
    let mut line_no = 1usize;
    let mut last_counted = 0usize;

    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\n' {
            i += 1;
            continue;
        }
        if b == b'!' || b == b'[' {
            let is_image = b == b'!';
            let bracket_start = if is_image { i + 1 } else { i };
            if is_image && bytes.get(bracket_start) != Some(&b'[') {
                i += 1;
                continue;
            }
            if let Some((alt_end, target)) = parse_md_link(source, bracket_start) {
                // Compute the line number at `i` by counting newlines up to this offset.
                line_no += source[last_counted..i].matches('\n').count();
                last_counted = i;

                if !should_skip_target(&target) {
                    let resolved_rel = strip_anchor_and_query(&target);
                    let resolved = base.join(resolved_rel);
                    if !resolved.exists() {
                        let code = if is_image {
                            "E_LINT_DEAD_ASSET"
                        } else {
                            "E_LINT_DEAD_LOCAL_LINK"
                        };
                        diags.push(Diagnostic::error(
                            rel.to_path_buf(),
                            code,
                            format!(
                                "dead {} target `{target}` at line {line_no}",
                                if is_image { "asset" } else { "local link" }
                            ),
                        ));
                    }
                }
                i = alt_end;
                continue;
            }
        }
        i += 1;
    }

    diags
}

/// If `source[start..]` begins a markdown link/image (`[text](target)`), return the byte
/// offset just past the closing `)` and the extracted `target`. Returns `None` if `start`
/// does not point at `[` or the construct is not well-formed on one logical span.
fn parse_md_link(source: &str, start: usize) -> Option<(usize, String)> {
    let bytes = source.as_bytes();
    if bytes.get(start) != Some(&b'[') {
        return None;
    }
    // Find the matching `]` for the text/alt part (no nested brackets handled — good enough
    // for the content this validator sees).
    let mut j = start + 1;
    while j < bytes.len() && bytes[j] != b']' && bytes[j] != b'\n' {
        j += 1;
    }
    if bytes.get(j) != Some(&b']') {
        return None;
    }
    let close_bracket = j;
    if bytes.get(close_bracket + 1) != Some(&b'(') {
        return None;
    }
    let mut k = close_bracket + 2;
    while k < bytes.len() && bytes[k] != b')' && bytes[k] != b'\n' {
        k += 1;
    }
    if bytes.get(k) != Some(&b')') {
        return None;
    }
    let target = &source[close_bracket + 2..k];
    Some((k + 1, target.to_string()))
}

/// `true` if `target` is an absolute URL, protocol-relative, or a pure in-page anchor — all
/// out of scope for local existence checking.
fn should_skip_target(target: &str) -> bool {
    let trimmed = target.trim();
    if trimmed.is_empty() {
        return true;
    }
    trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
        || trimmed.starts_with("mailto:")
        || trimmed.starts_with("//")
        || trimmed.starts_with('#')
}

/// Strip a trailing `#anchor` and/or `?query` suffix from a link target, returning the bare
/// path portion to resolve on disk.
fn strip_anchor_and_query(target: &str) -> &str {
    let target = target.trim();
    let end = target.find(['#', '?']).unwrap_or(target.len());
    &target[..end]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Severity;
    use std::path::PathBuf;

    fn rel() -> PathBuf {
        PathBuf::from("blog/published/post.mdx")
    }

    // -----------------------------------------------------------------
    // lint_code_blocks
    // -----------------------------------------------------------------

    #[test]
    fn tagged_fence_produces_no_diagnostic() {
        let source = "text\n```rust\nfn main() {}\n```\nmore text\n";
        let diags = lint_code_blocks(&rel(), source);
        assert!(diags.is_empty(), "tagged fence should not warn: {diags:?}");
    }

    #[test]
    fn untagged_fence_produces_warning_at_correct_line() {
        let source = "text\n```\nfn main() {}\n```\n";
        let diags = lint_code_blocks(&rel(), source);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].severity, Severity::Warning);
        assert_eq!(diags[0].locator, "W_LINT_UNTAGGED_CODE_BLOCK");
        assert!(diags[0].message.contains("line 2"));
    }

    #[test]
    fn tilde_fence_is_recognized() {
        let source = "text\n~~~\ncode\n~~~\n";
        let diags = lint_code_blocks(&rel(), source);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].locator, "W_LINT_UNTAGGED_CODE_BLOCK");
    }

    #[test]
    fn tilde_fence_with_tag_is_clean() {
        let source = "text\n~~~bash\necho hi\n~~~\n";
        let diags = lint_code_blocks(&rel(), source);
        assert!(diags.is_empty());
    }

    #[test]
    fn nested_fence_inside_open_block_not_double_counted() {
        // A ``` block containing lines that look like a ~~~ fence should not open/close a
        // second block or produce a second diagnostic.
        let source = "```md\nSome example:\n~~~\nnested\n~~~\n```\n";
        let diags = lint_code_blocks(&rel(), source);
        assert!(
            diags.is_empty(),
            "outer fence is tagged and inner fence chars are inert: {diags:?}"
        );
    }

    #[test]
    fn untagged_outer_fence_with_nested_marker_counts_once() {
        let source = "```\nSome example:\n```\nmore\n```\nend\n```\n";
        // Two full ``` blocks in sequence, both untagged -> two warnings, not more.
        let diags = lint_code_blocks(&rel(), source);
        assert_eq!(diags.len(), 2);
    }

    // -----------------------------------------------------------------
    // lint_local_links
    // -----------------------------------------------------------------

    #[test]
    fn absolute_url_is_skipped() {
        let source = "[site](https://example.com/page)\n";
        let file = Path::new("/tmp/does-not-matter/post.mdx");
        let diags = lint_local_links(file, &rel(), source);
        assert!(diags.is_empty());
    }

    #[test]
    fn anchor_only_link_is_skipped() {
        let source = "[jump](#section-one)\n";
        let file = Path::new("/tmp/does-not-matter/post.mdx");
        let diags = lint_local_links(file, &rel(), source);
        assert!(diags.is_empty());
    }

    #[test]
    fn mailto_and_protocol_relative_are_skipped() {
        let source = "[email](mailto:a@b.com) and [cdn](//cdn.example.com/x.png)\n";
        let file = Path::new("/tmp/does-not-matter/post.mdx");
        let diags = lint_local_links(file, &rel(), source);
        assert!(diags.is_empty());
    }

    #[test]
    fn dead_relative_link_is_reported() {
        let dir = crate::testsupport::unique_temp_dir("mev-lint-links");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("post.mdx");
        let source = "[missing](./nope.mdx)\n";
        let diags = lint_local_links(&file, &rel(), source);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].severity, Severity::Error);
        assert_eq!(diags[0].locator, "E_LINT_DEAD_LOCAL_LINK");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn live_relative_link_is_clean() {
        let dir = crate::testsupport::unique_temp_dir("mev-lint-links-live");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("post.mdx");
        std::fs::write(dir.join("other.mdx"), "hello").unwrap();
        let source = "[present](./other.mdx)\n";
        let diags = lint_local_links(&file, &rel(), source);
        assert!(diags.is_empty(), "{diags:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn link_with_anchor_suffix_resolves_against_stripped_path() {
        let dir = crate::testsupport::unique_temp_dir("mev-lint-links-anchor-suffix");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("post.mdx");
        std::fs::write(dir.join("other.mdx"), "hello").unwrap();
        let source = "[present](./other.mdx#section)\n";
        let diags = lint_local_links(&file, &rel(), source);
        assert!(diags.is_empty(), "{diags:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn dead_image_asset_is_reported() {
        let dir = crate::testsupport::unique_temp_dir("mev-lint-assets");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("post.mdx");
        let source = "![alt](./missing.png)\n";
        let diags = lint_local_links(&file, &rel(), source);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].severity, Severity::Error);
        assert_eq!(diags[0].locator, "E_LINT_DEAD_ASSET");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn live_image_asset_is_clean() {
        let dir = crate::testsupport::unique_temp_dir("mev-lint-assets-live");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("post.mdx");
        std::fs::write(dir.join("pic.png"), [0u8; 4]).unwrap();
        let source = "![alt](./pic.png)\n";
        let diags = lint_local_links(&file, &rel(), source);
        assert!(diags.is_empty(), "{diags:?}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
