//! Voice tripwire scanner (Phase 12, Block C, Task 2).
//!
//! [`check_voice`] scans prose lines for the banned phrases named in `learn-ai/CLAUDE.md`'s
//! "Voice and tone" bullet — loaded, per [`crate::learn_ai::voice_tells`], from
//! `data/voice-tells.toml` (or a caller-supplied override list, which is what proves the list
//! is genuinely data-driven rather than hardcoded here).
//!
//! Every diagnostic this module emits is `Severity::Warning` carrying `W_VOICE_TELL` in
//! [`Diagnostic::locator`] — there is no code path here that produces an error. A false
//! positive from this scanner must never be able to block a push; task 3 wires that guarantee
//! through to `BlogValidator`'s report.
//!
//! Four kinds of content are exempt from matching: fenced code blocks, inline code spans,
//! blockquote lines, and the YAML frontmatter block. The fenced-code-block exemption reuses
//! [`crate::learn_ai::lint::lines_in_code_fence`] — the same fence-tracking logic
//! `lint_local_links` uses — rather than a second, independently-drifting scanner.

use std::path::Path;

use crate::Diagnostic;
use crate::learn_ai::lint::lines_in_code_fence;
use crate::learn_ai::voice_tells::VoiceTell;

/// Scan `source` (the content of file `rel`) for every phrase in `tells`, outside code and
/// quotation, and return one `W_VOICE_TELL` warning per match.
///
/// Matching is case-insensitive and respects word boundaries on both sides of the phrase — a
/// phrase must not fire when it is embedded in a longer hyphenated token (e.g.
/// `production-ready` must not fire inside `non-production-ready-build`), and a match can
/// never span a line break because the scan is line-by-line.
pub fn check_voice(rel: &Path, source: &str, tells: &[VoiceTell]) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    if tells.is_empty() {
        return diags;
    }

    let fence_flags = lines_in_code_fence(source);
    let frontmatter_flags = frontmatter_lines(source);

    for (idx, line) in source.lines().enumerate() {
        let line_no = idx + 1;
        if fence_flags.get(idx).copied().unwrap_or(false) {
            continue;
        }
        if frontmatter_flags.get(idx).copied().unwrap_or(false) {
            continue;
        }
        if line.trim_start().starts_with('>') {
            continue;
        }

        let masked = mask_inline_code(line);
        let masked_lower = masked.to_lowercase();

        for tell in tells {
            let phrase_lower = tell.phrase.to_lowercase();
            if phrase_lower.is_empty() {
                continue;
            }
            for (start, _) in masked_lower.match_indices(&phrase_lower) {
                let end = start + phrase_lower.len();
                let before_ok = masked_lower[..start]
                    .chars()
                    .next_back()
                    .map(|c| !is_boundary_char(c))
                    .unwrap_or(true);
                let after_ok = masked_lower[end..]
                    .chars()
                    .next()
                    .map(|c| !is_boundary_char(c))
                    .unwrap_or(true);
                if before_ok && after_ok {
                    diags.push(Diagnostic::warning(
                        rel.to_path_buf(),
                        "W_VOICE_TELL",
                        format!("voice tell \"{}\" at line {line_no}", tell.phrase),
                    ));
                }
            }
        }
    }

    diags
}

/// A character that can be part of a longer token — letters, digits, underscore, and hyphen.
/// Used on both sides of a phrase match so `production-ready` cannot fire as a substring of a
/// longer hyphenated token such as `non-production-ready-build`.
fn is_boundary_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-'
}

/// Replace the content of every backtick-delimited inline code span (and the backticks
/// themselves) with spaces, preserving line length/byte positions so line numbers stay
/// accurate. A phrase that only appears inside an inline code span therefore cannot match.
fn mask_inline_code(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut in_code = false;
    for c in line.chars() {
        if c == '`' {
            in_code = !in_code;
            out.push(' ');
        } else if in_code {
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    out
}

/// Return, per 1-indexed line (0-indexed in the returned `Vec`), whether that line sits inside
/// — or is a delimiter of — the leading YAML frontmatter block (`---` ... `---` at the very top
/// of the file). Returns all-`false` when the file has no frontmatter block.
fn frontmatter_lines(source: &str) -> Vec<bool> {
    let lines: Vec<&str> = source.lines().collect();
    let mut flags = vec![false; lines.len()];

    if lines.first().map(|l| l.trim_end()) != Some("---") {
        return flags;
    }

    flags[0] = true;
    for (i, line) in lines.iter().enumerate().skip(1) {
        flags[i] = true;
        if line.trim_end() == "---" {
            break;
        }
    }
    flags
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Severity;
    use crate::learn_ai::voice_tells::default_tells;
    use std::path::PathBuf;

    fn rel() -> PathBuf {
        PathBuf::from("blog/published/post.mdx")
    }

    #[test]
    fn each_seeded_phrase_fires_in_prose() {
        let tells = default_tells();
        for tell in tells {
            let source = format!("This library is {}.\n", tell.phrase);
            let diags = check_voice(&rel(), &source, tells);
            assert_eq!(
                diags.len(),
                1,
                "expected exactly one hit for {:?}, got {diags:?}",
                tell.phrase
            );
            assert_eq!(diags[0].locator, "W_VOICE_TELL");
            assert!(diags[0].message.contains(&tell.phrase));
            assert!(diags[0].message.contains("line 1"));
        }
    }

    #[test]
    fn phrase_inside_fenced_code_block_is_exempt() {
        let source = "```\nThis is production-ready code.\n```\n";
        let diags = check_voice(&rel(), source, default_tells());
        assert!(diags.is_empty(), "fenced content must be exempt: {diags:?}");
    }

    #[test]
    fn phrase_inside_inline_code_span_is_exempt() {
        let source = "See `production-ready` in the docs.\n";
        let diags = check_voice(&rel(), source, default_tells());
        assert!(diags.is_empty(), "inline code must be exempt: {diags:?}");
    }

    #[test]
    fn phrase_inside_blockquote_is_exempt() {
        let source = "> This claims to be production-ready.\n";
        let diags = check_voice(&rel(), source, default_tells());
        assert!(diags.is_empty(), "blockquote must be exempt: {diags:?}");
    }

    #[test]
    fn indented_blockquote_is_also_exempt() {
        let source = "   > production-ready, allegedly.\n";
        let diags = check_voice(&rel(), source, default_tells());
        assert!(
            diags.is_empty(),
            "blockquote with leading whitespace must be exempt: {diags:?}"
        );
    }

    #[test]
    fn phrase_inside_frontmatter_is_exempt() {
        let source = "---\ntitle: production-ready things\ndescription: x\n---\n\nBody text.\n";
        let diags = check_voice(&rel(), source, default_tells());
        assert!(diags.is_empty(), "frontmatter must be exempt: {diags:?}");
    }

    #[test]
    fn match_is_case_insensitive() {
        let source = "This is PRODUCTION-READY software.\n";
        let diags = check_voice(&rel(), source, default_tells());
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].severity, Severity::Warning);
    }

    #[test]
    fn phrase_split_across_a_line_break_does_not_match() {
        let source = "This is production-\nready software.\n";
        let diags = check_voice(&rel(), source, default_tells());
        assert!(
            diags.is_empty(),
            "a phrase split by a line break must not match: {diags:?}"
        );
    }

    #[test]
    fn phrase_embedded_in_a_longer_hyphenated_token_does_not_match() {
        let source = "This is non-production-ready-build software.\n";
        let diags = check_voice(&rel(), source, default_tells());
        assert!(
            diags.is_empty(),
            "a phrase embedded in a longer hyphenated token must not match: {diags:?}"
        );
    }

    #[test]
    fn phrase_absent_from_shipped_list_fires_via_override_list() {
        // Proves adding a tell needs no Rust change: a phrase that is NOT in the shipped
        // default list still fires when supplied through an override list.
        let shipped: Vec<&str> = default_tells().iter().map(|t| t.phrase.as_str()).collect();
        let custom_phrase = "bleeding-edge synergy";
        assert!(
            !shipped.contains(&custom_phrase),
            "test fixture phrase must not already be in the shipped list"
        );

        let overrides = vec![VoiceTell {
            phrase: custom_phrase.to_string(),
            note: "test override".to_string(),
        }];
        let source = format!("We embrace {custom_phrase} here.\n");
        let diags = check_voice(&rel(), &source, &overrides);
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].locator, "W_VOICE_TELL");
    }

    #[test]
    fn every_diagnostic_this_module_emits_is_warning_severity() {
        let source = "production-ready, game-changing, actually bites you\n";
        let diags = check_voice(&rel(), source, default_tells());
        assert!(!diags.is_empty(), "expected at least one hit: {diags:?}");
        for diag in &diags {
            assert_eq!(
                diag.severity,
                Severity::Warning,
                "no diagnostic from this module may be Severity::Error: {diag:?}"
            );
        }
    }

    #[test]
    fn empty_tells_list_produces_no_diagnostics() {
        let source = "production-ready and game-changing everywhere.\n";
        let diags = check_voice(&rel(), source, &[]);
        assert!(diags.is_empty(), "{diags:?}");
    }

    #[test]
    fn multiple_seeded_phrases_on_one_line_each_produce_a_diagnostic() {
        let source = "It is production-ready and game-changing.\n";
        let diags = check_voice(&rel(), source, default_tells());
        assert_eq!(diags.len(), 2, "{diags:?}");
        assert!(diags.iter().all(|d| d.severity == Severity::Warning));
    }
}
