//! Shared helper functions used by multiple validators.
//!
//! These are string-manipulation and format-checking utilities that do not belong to any single
//! consumer (learn-ai, OKF, …) and carry no domain types. Each helper is `pub(crate)` so all
//! internal modules can import it without exposing it as part of the crate's public API.

/// Extract the raw YAML text from a leading `---\n…\n---` frontmatter block.
///
/// Returns `None` when the file does not start with `---` (no frontmatter), or when the
/// closing `---` fence is never found (unterminated block).
pub(crate) fn extract_frontmatter(contents: &str) -> Option<&str> {
    let rest = contents.strip_prefix("---")?;
    // Accept `---\n` or `---\r\n` as the opening fence line.
    let rest = rest
        .strip_prefix('\n')
        .or_else(|| rest.strip_prefix("\r\n"))?;

    // Special case: closing fence immediately follows opening fence (empty frontmatter).
    for empty_pat in ["---\n", "---\r\n", "---"] {
        if rest.starts_with(empty_pat) {
            return Some("");
        }
    }

    // General case: find the closing `---` on its own line (preceded by a newline).
    for end_pat in ["\n---\n", "\n---\r\n", "\n---"] {
        if let Some(yaml_end) = rest.find(end_pat) {
            return Some(&rest[..yaml_end]);
        }
    }

    None
}

/// Return the trimmed-non-empty borrow of an optional string, or `None` if absent/blank.
pub(crate) fn non_empty(value: &Option<String>) -> Option<&str> {
    match value {
        Some(s) if !s.trim().is_empty() => Some(s),
        _ => None,
    }
}

/// `true` if `s` matches `^[a-z0-9]+(-[a-z0-9]+)*$` (kebab-case), without the `regex` crate.
pub(crate) fn is_kebab_case(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    // Each hyphen-delimited segment must be non-empty and all `[a-z0-9]` — this rejects
    // leading/trailing hyphens and `--` runs (which produce an empty segment when split).
    s.split('-')
        .all(|seg| !seg.is_empty() && seg.chars().all(|c| matches!(c, 'a'..='z' | '0'..='9')))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_frontmatter_helper() {
        // Normal case.
        let body = "---\ntitle: T\n---\nbody";
        assert_eq!(extract_frontmatter(body), Some("title: T"));

        // Unterminated: returns None.
        let unterminated = "---\ntitle: T\n";
        assert_eq!(extract_frontmatter(unterminated), None);

        // No leading fence: returns None.
        let no_fence = "title: T\n---\n";
        assert_eq!(extract_frontmatter(no_fence), None);

        // Empty frontmatter block.
        let empty_block = "---\n---\nbody";
        assert_eq!(extract_frontmatter(empty_block), Some(""));
    }

    #[test]
    fn kebab_case_helper() {
        assert!(is_kebab_case("intro-to-mcp"));
        assert!(is_kebab_case("abc123"));
        assert!(!is_kebab_case("Intro"));
        assert!(!is_kebab_case("-leading"));
        assert!(!is_kebab_case("trailing-"));
        assert!(!is_kebab_case("double--hyphen"));
        assert!(!is_kebab_case("under_score"));
        assert!(!is_kebab_case(""));
    }

    #[test]
    fn non_empty_helper() {
        assert_eq!(non_empty(&Some("hello".to_string())), Some("hello"));
        assert_eq!(non_empty(&Some("  ".to_string())), None);
        assert_eq!(non_empty(&None), None);
        assert_eq!(non_empty(&Some("".to_string())), None);
        // Trimmed value is returned as-is (not trimmed), but check it's truthy.
        assert_eq!(non_empty(&Some("  x  ".to_string())), Some("  x  "));
    }
}
