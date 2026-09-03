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

use std::path::{Path, PathBuf};

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
/// emit a diagnostic for every target that does not exist.
///
/// Absolute URLs (`http://`, `https://`, `mailto:`) and protocol-relative (`//`) targets are
/// skipped — no network calls, ever. Pure in-page anchors (`#...`) are skipped.
///
/// A **site-absolute** target (a single leading `/`, e.g. `/en/blog/x` or `/learn/y`) is a
/// Next.js route, not a filesystem path. Two distinct outcomes follow, and they must not be
/// collapsed into one: a route shape [`resolve_route`]'s seven mapping rules recognize is
/// resolved to a content path and checked for existence; a route shape [`known_invalid_learn_route`]
/// recognizes as **known invalid** (e.g. `/learn/<slug>`, which has no matching Next.js route
/// even though the aliased content file exists on disk) is reported directly, naming the
/// correct route in the diagnostic. Anything else — a route shape neither function models, or
/// a `content_root` of `None` — is skipped silently rather than reported: mev must not assert
/// on routes it does not model.
///
/// Everything else (a genuinely relative target) is resolved relative to `file`'s parent
/// directory, after stripping any trailing `#anchor` or `?query` suffix, and checked for
/// existence: `E_LINT_DEAD_ASSET` for an image reference, `E_LINT_DEAD_LOCAL_LINK` for a link.
///
/// Bracket/paren text that merely *looks like* a markdown link but sits inside a fenced code
/// block (e.g. `results[node](results)` in a Python snippet) is not a link and is skipped —
/// discovered as a live-corpus false positive alongside the route-mapping gap this function's
/// `content_root` parameter fixes.
pub fn lint_local_links(
    file: &Path,
    rel: &Path,
    source: &str,
    content_root: Option<&Path>,
) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    let base = file.parent().unwrap_or_else(|| Path::new("."));
    let fence_lines = lines_in_code_fence(source);

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
            // A `[` immediately preceded by an identifier character (letter/digit/`_`) is
            // array/object indexing in code — e.g. `results[node](results)` in a Python
            // snippet — not markdown link syntax, which is never glued to a preceding
            // identifier. Real markdown links are preceded by whitespace, punctuation, or
            // start-of-text. This is what keeps code embedded via JSX component props (e.g.
            // `<CodeExample code={\`...\`} />`, which carries no ``` fence for
            // `lines_in_code_fence` to see) from producing phantom dead-link diagnostics.
            let glued_to_identifier =
                i > 0 && matches!(bytes[i - 1], b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_');
            if !is_image && glued_to_identifier {
                i += 1;
                continue;
            }
            if let Some((alt_end, target)) = parse_md_link(source, bracket_start) {
                // Compute the line number at `i` by counting newlines up to this offset.
                line_no += source[last_counted..i].matches('\n').count();
                last_counted = i;

                let in_fence = fence_lines.get(line_no - 1).copied().unwrap_or(false);
                if !in_fence && !should_skip_target(&target) {
                    let resolved_rel = strip_anchor_and_query(&target);
                    let resolution = if resolved_rel.starts_with('/') {
                        // Site-absolute: route-aware resolution. A route shape that is
                        // *known invalid* (e.g. `/learn/<slug>`, which has no matching
                        // Next.js route) is reported directly, naming the correct route. A
                        // route shape `resolve_route` does not model at all, or a
                        // `content_root` of `None`, is skipped silently — mev must not
                        // assert on routes it does not model.
                        content_root.and_then(|root| {
                            if let Some(correct_route) = known_invalid_learn_route(resolved_rel) {
                                Some(RouteResolution::KnownInvalid(correct_route))
                            } else {
                                resolve_route(resolved_rel, root).map(RouteResolution::Path)
                            }
                        })
                    } else {
                        Some(RouteResolution::Path(base.join(resolved_rel)))
                    };

                    let dead_message = match &resolution {
                        Some(RouteResolution::Path(p)) if !p.exists() => Some(format!(
                            "dead {} target `{target}` at line {line_no}",
                            if is_image { "asset" } else { "local link" }
                        )),
                        Some(RouteResolution::KnownInvalid(correct_route)) => Some(format!(
                            "dead {} target `{target}` at line {line_no}: `{resolved_rel}` is \
                             not a valid route, use `{correct_route}` instead",
                            if is_image { "asset" } else { "local link" }
                        )),
                        _ => None,
                    };

                    if let Some(message) = dead_message {
                        let code = if is_image {
                            "E_LINT_DEAD_ASSET"
                        } else {
                            "E_LINT_DEAD_LOCAL_LINK"
                        };
                        diags.push(Diagnostic::error(rel.to_path_buf(), code, message));
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

/// Return, per 1-indexed line (0-indexed in the returned `Vec`), whether that line sits
/// inside — or is a fence delimiter of — a fenced code block, using the same fence-tracking
/// rules as [`lint_code_blocks`] (``` and ~~~, honouring longer fences, no double-counting a
/// same-character fence nested inside an already-open block of the other character).
/// [`lint_local_links`] uses this to avoid mistaking link-shaped code content (e.g.
/// `results[node](results)` in a Python snippet) for a real markdown link.
pub(crate) fn lines_in_code_fence(source: &str) -> Vec<bool> {
    let mut in_fence = Vec::new();
    let mut open_fence: Option<(char, usize)> = None;

    for line in source.lines() {
        let trimmed = line.trim_start();
        let fence_char = trimmed.chars().next();
        let mut this_line_in_fence = open_fence.is_some();

        if let Some(fc) = fence_char
            && (fc == '`' || fc == '~')
        {
            let run_len = trimmed.chars().take_while(|&c| c == fc).count();
            if run_len >= 3 {
                let after_fence = &trimmed[run_len..];
                match open_fence {
                    None => {
                        open_fence = Some((fc, run_len));
                        this_line_in_fence = true; // the opening fence line itself
                    }
                    Some((open_char, open_len)) => {
                        if fc == open_char && run_len >= open_len && after_fence.trim().is_empty() {
                            open_fence = None;
                            this_line_in_fence = true; // the closing fence line itself
                        }
                    }
                }
            }
        }

        in_fence.push(this_line_in_fence);
    }

    in_fence
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

/// Map a site-absolute route (e.g. `/en/blog/x`, `/blog/x`, `/learn/paths/x`,
/// `/en/learn/paths/x`, `/learn/concepts/x`, `/pt-BR/learn/concepts/x`) to a content path under
/// `content_root`, per the **seven** mapping rules established for Task 8
/// (`planning/12.A-blog-module-linting/tasks.md` — route-aware resolution) and extended by
/// `planning/ticket-learn-link-mapping-masks-dead-links` for locale-prefixed learn routes and
/// `/learn/concepts/<slug>`. Returns `None` for any route shape none of these seven rules
/// cover — such a path is skipped silently by the caller, never reported, because mev must not
/// assert on routes it does not model. This is distinct from a **known-invalid** shape (e.g.
/// `/learn/<slug>`): that case is intercepted by [`known_invalid_learn_route`] before this
/// function is ever called, so it is reported rather than silently skipped or misresolved.
///
/// `route` must start with `/` (the caller strips the leading `/` here).
fn resolve_route(route: &str, content_root: &Path) -> Option<PathBuf> {
    let trimmed = route.trim_start_matches('/');
    let segments: Vec<&str> = trimmed.split('/').filter(|s| !s.is_empty()).collect();
    match segments.as_slice() {
        // /<locale>/blog/<slug>
        [locale, "blog", slug] if *locale == "en" => Some(
            content_root
                .join("blog/published")
                .join(format!("{slug}.mdx")),
        ),
        [locale, "blog", slug] if *locale == "pt-BR" => Some(
            content_root
                .join("blog/published/pt-BR")
                .join(format!("{slug}.mdx")),
        ),
        // /blog/<slug> (no locale segment)
        ["blog", slug] => Some(
            content_root
                .join("blog/published")
                .join(format!("{slug}.mdx")),
        ),
        // /<locale>/learn/paths/<slug>
        [locale, "learn", "paths", slug] if *locale == "en" || *locale == "pt-BR" => {
            Some(content_root.join("learn/paths").join(slug))
        }
        // /<locale>/learn/concepts/<slug>
        [locale, "learn", "concepts", slug] if *locale == "en" || *locale == "pt-BR" => {
            Some(content_root.join("learn/concepts").join(slug))
        }
        // /learn/paths/<slug>
        ["learn", "paths", slug] => Some(content_root.join("learn/paths").join(slug)),
        // /learn/concepts/<slug>
        ["learn", "concepts", slug] => Some(content_root.join("learn/concepts").join(slug)),
        _ => None,
    }
}

/// Outcome of resolving a site-absolute route: either it maps to a filesystem path to check
/// for existence, or the route shape itself is known to be invalid (distinct from a route
/// shape [`resolve_route`] simply does not model, which is skipped silently).
enum RouteResolution {
    /// A modeled route, resolved to the content path it should point at.
    Path(PathBuf),
    /// A route shape known not to exist in learn-ai's App Router tree (e.g. `/learn/<slug>`),
    /// carrying the correct route a reader should use instead.
    KnownInvalid(String),
}

/// Returns `Some(correct_route)` when `route` is `/learn/<slug>` — a shape that looks like a
/// valid content path (the file exists on disk at `content/learn/paths/<slug>`) but is **not**
/// a real Next.js route: there is no `[slug]` segment directly under `app/[locale]/learn/`,
/// only the two named children `learn/paths/<slug>` and `learn/concepts/<slug>`. This is what
/// lets [`lint_local_links`] report a diagnostic naming the correct route, rather than either
/// (a) silently skipping the link because it looks like an unmodeled shape, or (b) resolving
/// it to a path that happens to exist and reading it as clean.
///
/// `route` must start with `/` (the caller strips the leading `/` here).
fn known_invalid_learn_route(route: &str) -> Option<String> {
    let trimmed = route.trim_start_matches('/');
    let segments: Vec<&str> = trimmed.split('/').filter(|s| !s.is_empty()).collect();
    match segments.as_slice() {
        ["learn", slug] if *slug != "paths" && *slug != "concepts" => {
            Some(format!("/learn/paths/{slug}"))
        }
        _ => None,
    }
}

/// Derive the content root (the directory that contains both `blog/` and `learn/`) from a
/// `ContentValidator`'s `root` argument, so [`lint_local_links`] can resolve site-absolute
/// routes even though it only ever sees one subtree at a time.
///
/// - `<content>/blog/published` -> `<content>`
/// - `<content>/learn` -> `<content>`
/// - anything else -> `None` (the caller must then skip all absolute links rather than guess)
pub fn derive_content_root(root: &Path) -> Option<PathBuf> {
    let file_name = root.file_name()?.to_str()?;
    if file_name == "published" {
        let parent = root.parent()?;
        if parent.file_name().and_then(|s| s.to_str()) == Some("blog") {
            return parent.parent().map(Path::to_path_buf);
        }
        return None;
    }
    if file_name == "learn" {
        return root.parent().map(Path::to_path_buf);
    }
    None
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
        let diags = lint_local_links(file, &rel(), source, None);
        assert!(diags.is_empty());
    }

    #[test]
    fn anchor_only_link_is_skipped() {
        let source = "[jump](#section-one)\n";
        let file = Path::new("/tmp/does-not-matter/post.mdx");
        let diags = lint_local_links(file, &rel(), source, None);
        assert!(diags.is_empty());
    }

    #[test]
    fn mailto_and_protocol_relative_are_skipped() {
        let source = "[email](mailto:a@b.com) and [cdn](//cdn.example.com/x.png)\n";
        let file = Path::new("/tmp/does-not-matter/post.mdx");
        let diags = lint_local_links(file, &rel(), source, None);
        assert!(diags.is_empty());
    }

    #[test]
    fn dead_relative_link_is_reported() {
        let dir = crate::testsupport::unique_temp_dir("mev-lint-links");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("post.mdx");
        let source = "[missing](./nope.mdx)\n";
        let diags = lint_local_links(&file, &rel(), source, None);
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
        let diags = lint_local_links(&file, &rel(), source, None);
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
        let diags = lint_local_links(&file, &rel(), source, None);
        assert!(diags.is_empty(), "{diags:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn dead_image_asset_is_reported() {
        let dir = crate::testsupport::unique_temp_dir("mev-lint-assets");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("post.mdx");
        let source = "![alt](./missing.png)\n";
        let diags = lint_local_links(&file, &rel(), source, None);
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
        let diags = lint_local_links(&file, &rel(), source, None);
        assert!(diags.is_empty(), "{diags:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    // -----------------------------------------------------------------
    // Route-aware resolution for site-absolute links (Task 8)
    // -----------------------------------------------------------------

    /// Build a `<root>/blog/published/<slug>.mdx` (+ optional pt-BR),
    /// `<root>/learn/paths/<slug>`, and `<root>/learn/concepts/<slug>` fixture tree, returning
    /// the content root.
    fn route_fixture_root(prefix: &str) -> PathBuf {
        let dir = crate::testsupport::unique_temp_dir(prefix);
        std::fs::create_dir_all(dir.join("blog/published/pt-BR")).unwrap();
        std::fs::create_dir_all(dir.join("learn/paths/existing-path")).unwrap();
        std::fs::create_dir_all(dir.join("learn/concepts/existing-concept")).unwrap();
        std::fs::write(dir.join("blog/published/hello.mdx"), "hi").unwrap();
        std::fs::write(dir.join("blog/published/pt-BR/hello.mdx"), "oi").unwrap();
        dir
    }

    #[test]
    fn locale_blog_route_resolves_en() {
        let content_root = route_fixture_root("mev-lint-route-en");
        let file = content_root.join("blog/published/other.mdx");
        let source = "[link](/en/blog/hello)\n";
        let diags = lint_local_links(&file, &rel(), source, Some(&content_root));
        assert!(
            diags.is_empty(),
            "existing EN route should be clean: {diags:?}"
        );
        std::fs::remove_dir_all(&content_root).ok();
    }

    #[test]
    fn locale_blog_route_resolves_ptbr() {
        let content_root = route_fixture_root("mev-lint-route-ptbr");
        let file = content_root.join("blog/published/other.mdx");
        let source = "[link](/pt-BR/blog/hello)\n";
        let diags = lint_local_links(&file, &rel(), source, Some(&content_root));
        assert!(
            diags.is_empty(),
            "existing pt-BR route should be clean: {diags:?}"
        );
        std::fs::remove_dir_all(&content_root).ok();
    }

    #[test]
    fn locale_blog_route_missing_target_is_reported() {
        let content_root = route_fixture_root("mev-lint-route-en-missing");
        let file = content_root.join("blog/published/other.mdx");
        let source = "[link](/en/blog/does-not-exist)\n";
        let diags = lint_local_links(&file, &rel(), source, Some(&content_root));
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].locator, "E_LINT_DEAD_LOCAL_LINK");
        std::fs::remove_dir_all(&content_root).ok();
    }

    #[test]
    fn no_locale_blog_route_resolves() {
        let content_root = route_fixture_root("mev-lint-route-no-locale");
        let file = content_root.join("blog/published/other.mdx");
        let source = "[link](/blog/hello)\n";
        let diags = lint_local_links(&file, &rel(), source, Some(&content_root));
        assert!(diags.is_empty(), "{diags:?}");
        std::fs::remove_dir_all(&content_root).ok();
    }

    #[test]
    fn no_locale_blog_route_missing_target_is_reported() {
        let content_root = route_fixture_root("mev-lint-route-no-locale-missing");
        let file = content_root.join("blog/published/other.mdx");
        let source = "[link](/blog/nope)\n";
        let diags = lint_local_links(&file, &rel(), source, Some(&content_root));
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].locator, "E_LINT_DEAD_LOCAL_LINK");
        std::fs::remove_dir_all(&content_root).ok();
    }

    #[test]
    fn learn_paths_route_resolves() {
        let content_root = route_fixture_root("mev-lint-route-learn-paths");
        let file = content_root.join("learn/some-module.mdx");
        let source = "[link](/learn/paths/existing-path)\n";
        let diags = lint_local_links(&file, &rel(), source, Some(&content_root));
        assert!(diags.is_empty(), "{diags:?}");
        std::fs::remove_dir_all(&content_root).ok();
    }

    #[test]
    fn learn_paths_route_missing_target_is_reported() {
        let content_root = route_fixture_root("mev-lint-route-learn-paths-missing");
        let file = content_root.join("learn/some-module.mdx");
        let source = "[link](/learn/paths/nope)\n";
        let diags = lint_local_links(&file, &rel(), source, Some(&content_root));
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].locator, "E_LINT_DEAD_LOCAL_LINK");
        std::fs::remove_dir_all(&content_root).ok();
    }

    #[test]
    fn locale_learn_paths_route_resolves_en() {
        let content_root = route_fixture_root("mev-lint-route-learn-paths-en");
        let file = content_root.join("learn/some-module.mdx");
        let source = "[link](/en/learn/paths/existing-path)\n";
        let diags = lint_local_links(&file, &rel(), source, Some(&content_root));
        assert!(diags.is_empty(), "{diags:?}");
        std::fs::remove_dir_all(&content_root).ok();
    }

    #[test]
    fn locale_learn_paths_route_resolves_ptbr() {
        let content_root = route_fixture_root("mev-lint-route-learn-paths-ptbr");
        let file = content_root.join("learn/some-module.mdx");
        let source = "[link](/pt-BR/learn/paths/existing-path)\n";
        let diags = lint_local_links(&file, &rel(), source, Some(&content_root));
        assert!(diags.is_empty(), "{diags:?}");
        std::fs::remove_dir_all(&content_root).ok();
    }

    #[test]
    fn locale_learn_paths_route_missing_target_is_reported_en() {
        let content_root = route_fixture_root("mev-lint-route-learn-paths-en-missing");
        let file = content_root.join("learn/some-module.mdx");
        let source = "[link](/en/learn/paths/nope)\n";
        let diags = lint_local_links(&file, &rel(), source, Some(&content_root));
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].locator, "E_LINT_DEAD_LOCAL_LINK");
        std::fs::remove_dir_all(&content_root).ok();
    }

    #[test]
    fn locale_learn_paths_route_missing_target_is_reported_ptbr() {
        let content_root = route_fixture_root("mev-lint-route-learn-paths-ptbr-missing");
        let file = content_root.join("learn/some-module.mdx");
        let source = "[link](/pt-BR/learn/paths/nope)\n";
        let diags = lint_local_links(&file, &rel(), source, Some(&content_root));
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].locator, "E_LINT_DEAD_LOCAL_LINK");
        std::fs::remove_dir_all(&content_root).ok();
    }

    #[test]
    fn learn_concepts_route_resolves() {
        let content_root = route_fixture_root("mev-lint-route-learn-concepts");
        let file = content_root.join("learn/some-module.mdx");
        let source = "[link](/learn/concepts/existing-concept)\n";
        let diags = lint_local_links(&file, &rel(), source, Some(&content_root));
        assert!(diags.is_empty(), "{diags:?}");
        std::fs::remove_dir_all(&content_root).ok();
    }

    #[test]
    fn learn_concepts_route_missing_target_is_reported() {
        let content_root = route_fixture_root("mev-lint-route-learn-concepts-missing");
        let file = content_root.join("learn/some-module.mdx");
        let source = "[link](/learn/concepts/nope)\n";
        let diags = lint_local_links(&file, &rel(), source, Some(&content_root));
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].locator, "E_LINT_DEAD_LOCAL_LINK");
        std::fs::remove_dir_all(&content_root).ok();
    }

    #[test]
    fn locale_learn_concepts_route_resolves_en() {
        let content_root = route_fixture_root("mev-lint-route-learn-concepts-en");
        let file = content_root.join("learn/some-module.mdx");
        let source = "[link](/en/learn/concepts/existing-concept)\n";
        let diags = lint_local_links(&file, &rel(), source, Some(&content_root));
        assert!(diags.is_empty(), "{diags:?}");
        std::fs::remove_dir_all(&content_root).ok();
    }

    #[test]
    fn locale_learn_concepts_route_resolves_ptbr() {
        let content_root = route_fixture_root("mev-lint-route-learn-concepts-ptbr");
        let file = content_root.join("learn/some-module.mdx");
        let source = "[link](/pt-BR/learn/concepts/existing-concept)\n";
        let diags = lint_local_links(&file, &rel(), source, Some(&content_root));
        assert!(diags.is_empty(), "{diags:?}");
        std::fs::remove_dir_all(&content_root).ok();
    }

    #[test]
    fn locale_learn_concepts_route_missing_target_is_reported_en() {
        let content_root = route_fixture_root("mev-lint-route-learn-concepts-en-missing");
        let file = content_root.join("learn/some-module.mdx");
        let source = "[link](/en/learn/concepts/nope)\n";
        let diags = lint_local_links(&file, &rel(), source, Some(&content_root));
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].locator, "E_LINT_DEAD_LOCAL_LINK");
        std::fs::remove_dir_all(&content_root).ok();
    }

    #[test]
    fn locale_learn_concepts_route_missing_target_is_reported_ptbr() {
        let content_root = route_fixture_root("mev-lint-route-learn-concepts-ptbr-missing");
        let file = content_root.join("learn/some-module.mdx");
        let source = "[link](/pt-BR/learn/concepts/nope)\n";
        let diags = lint_local_links(&file, &rel(), source, Some(&content_root));
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].locator, "E_LINT_DEAD_LOCAL_LINK");
        std::fs::remove_dir_all(&content_root).ok();
    }

    #[test]
    fn learn_shorthand_route_is_reported_as_dead() {
        let content_root = route_fixture_root("mev-lint-route-learn-short");
        let file = content_root.join("learn/some-module.mdx");
        let source = "[link](/learn/existing-path)\n";
        let diags = lint_local_links(&file, &rel(), source, Some(&content_root));
        // `/learn/existing-path` is not a real route even though
        // `content/learn/paths/existing-path` exists on disk — the naive alias would read
        // this as clean, which is exactly the regression this ticket closes.
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].locator, "E_LINT_DEAD_LOCAL_LINK");
        assert!(
            diags[0].message.contains("/learn/paths/existing-path"),
            "diagnostic must name the correct route: {:?}",
            diags[0].message
        );
        std::fs::remove_dir_all(&content_root).ok();
    }

    #[test]
    fn unmapped_absolute_path_is_skipped_silently() {
        let content_root = route_fixture_root("mev-lint-route-unmapped");
        let file = content_root.join("blog/published/other.mdx");
        let source = "[link](/services) and [about](/about)\n";
        let diags = lint_local_links(&file, &rel(), source, Some(&content_root));
        assert!(
            diags.is_empty(),
            "unmapped absolute paths must be skipped, not reported: {diags:?}"
        );
        std::fs::remove_dir_all(&content_root).ok();
    }

    #[test]
    fn absolute_path_skipped_when_content_root_is_none() {
        let dir = crate::testsupport::unique_temp_dir("mev-lint-route-no-root");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("post.mdx");
        let source = "[link](/en/blog/anything)\n";
        let diags = lint_local_links(&file, &rel(), source, None);
        assert!(
            diags.is_empty(),
            "content_root: None must skip all absolute links: {diags:?}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn relative_link_behaviour_is_unregressed_with_content_root_present() {
        let content_root = route_fixture_root("mev-lint-route-relative-unregressed");
        let file = content_root.join("blog/published/other.mdx");
        std::fs::write(content_root.join("blog/published/sibling.mdx"), "x").unwrap();
        let source = "[dead](./missing.mdx) and [live](./sibling.mdx)\n";
        let diags = lint_local_links(&file, &rel(), source, Some(&content_root));
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].locator, "E_LINT_DEAD_LOCAL_LINK");
        std::fs::remove_dir_all(&content_root).ok();
    }

    // -----------------------------------------------------------------
    // derive_content_root
    // -----------------------------------------------------------------

    #[test]
    fn derive_content_root_from_blog_published() {
        let root = PathBuf::from("../learn-ai/content/blog/published");
        assert_eq!(
            derive_content_root(&root),
            Some(PathBuf::from("../learn-ai/content"))
        );
    }

    #[test]
    fn derive_content_root_from_learn() {
        let root = PathBuf::from("../learn-ai/content/learn");
        assert_eq!(
            derive_content_root(&root),
            Some(PathBuf::from("../learn-ai/content"))
        );
    }

    #[test]
    fn derive_content_root_returns_none_for_unrecognized_root() {
        let root = PathBuf::from("/some/other/tree");
        assert_eq!(derive_content_root(&root), None);
    }

    // -----------------------------------------------------------------
    // Link-shaped code content is not a real link (live-corpus false positive)
    // -----------------------------------------------------------------

    #[test]
    fn bracket_glued_to_identifier_is_not_treated_as_a_link() {
        // `results[node](results)` — array indexing followed by a call, not markdown link
        // syntax. `results` does not exist on disk, so a false positive would report it.
        let dir = crate::testsupport::unique_temp_dir("mev-lint-glued-identifier");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("post.mdx");
        let source = "results[node] = tasks[node](results)  # not a markdown link\n";
        let diags = lint_local_links(&file, &rel(), source, None);
        assert!(diags.is_empty(), "{diags:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn bracket_not_glued_to_identifier_is_still_a_real_link() {
        // A genuine markdown link preceded by whitespace is unaffected by the heuristic above.
        let dir = crate::testsupport::unique_temp_dir("mev-lint-not-glued");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("post.mdx");
        let source = "see [missing](./nope.mdx) for more\n";
        let diags = lint_local_links(&file, &rel(), source, None);
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].locator, "E_LINT_DEAD_LOCAL_LINK");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn link_shaped_content_inside_fenced_code_block_is_skipped() {
        let dir = crate::testsupport::unique_temp_dir("mev-lint-fence-skip");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("post.mdx");
        // Space before `[bad]` defeats the glued-identifier heuristic, so this specifically
        // exercises fence-awareness rather than the identifier heuristic.
        let source = "```md\nsee [bad](./missing.mdx) here\n```\n";
        let diags = lint_local_links(&file, &rel(), source, None);
        assert!(
            diags.is_empty(),
            "link-shaped text inside a fence must not be reported: {diags:?}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn lines_in_code_fence_marks_fence_delimiters_and_body_not_after() {
        let source = "before\n```\ninside\n```\nafter\n";
        let flags = lines_in_code_fence(source);
        assert_eq!(
            flags,
            vec![false, true, true, true, false],
            "before=false, open-fence-line=true, body=true, close-fence-line=true, after=false"
        );
    }
}
