//! Integration tests for the voice tripwire (Phase 12, Block C, Task 4).
//!
//! Covers each seeded phrase and each exemption end-to-end through
//! `mev::validate_blog` (not the pure `check_voice` function — that's `src/learn_ai/voice.rs`'s
//! job), using fixtures under `tests/fixtures/voice/`, plus a live-corpus test that doubles as
//! the operator report the block's acceptance criterion asks for.

use std::collections::BTreeMap;
use std::path::Path;

use mev::Severity;

/// The voice fixture tree checked into `tests/fixtures/voice/`.
fn voice_fixture_root() -> &'static Path {
    Path::new("tests/fixtures/voice")
}

fn voice_tell_diags_for_file<'a>(
    diags: &'a [mev::Diagnostic],
    file: &str,
) -> Vec<&'a mev::Diagnostic> {
    diags
        .iter()
        .filter(|d| d.file == Path::new(file) && d.locator == "W_VOICE_TELL")
        .collect()
}

// ---------------------------------------------------------------------------
// Each seeded phrase fires, end-to-end
// ---------------------------------------------------------------------------

#[test]
fn seeded_production_ready_fires_through_blog_validator() {
    let report = mev::validate_blog(voice_fixture_root()).expect("validate_blog");
    let hits = voice_tell_diags_for_file(&report.diagnostics, "seeded-production-ready.mdx");
    assert_eq!(hits.len(), 1, "{:?}", report.diagnostics);
    assert_eq!(hits[0].severity, Severity::Warning);
    assert!(hits[0].message.contains("production-ready"));
}

#[test]
fn seeded_game_changing_fires_through_blog_validator() {
    let report = mev::validate_blog(voice_fixture_root()).expect("validate_blog");
    let hits = voice_tell_diags_for_file(&report.diagnostics, "seeded-game-changing.mdx");
    assert_eq!(hits.len(), 1, "{:?}", report.diagnostics);
    assert_eq!(hits[0].severity, Severity::Warning);
    assert!(hits[0].message.contains("game-changing"));
}

#[test]
fn seeded_actually_bites_you_fires_through_blog_validator() {
    let report = mev::validate_blog(voice_fixture_root()).expect("validate_blog");
    let hits = voice_tell_diags_for_file(&report.diagnostics, "seeded-actually-bites-you.mdx");
    assert_eq!(hits.len(), 1, "{:?}", report.diagnostics);
    assert_eq!(hits[0].severity, Severity::Warning);
    assert!(hits[0].message.contains("actually bites you"));
}

// ---------------------------------------------------------------------------
// Each exemption, end-to-end
// ---------------------------------------------------------------------------

#[test]
fn phrase_in_fenced_code_block_is_exempt_end_to_end() {
    let report = mev::validate_blog(voice_fixture_root()).expect("validate_blog");
    let hits = voice_tell_diags_for_file(&report.diagnostics, "exempt-fenced-code.mdx");
    assert!(hits.is_empty(), "{hits:?}");
}

#[test]
fn phrase_in_inline_code_span_is_exempt_end_to_end() {
    let report = mev::validate_blog(voice_fixture_root()).expect("validate_blog");
    let hits = voice_tell_diags_for_file(&report.diagnostics, "exempt-inline-code.mdx");
    assert!(hits.is_empty(), "{hits:?}");
}

#[test]
fn phrase_in_blockquote_is_exempt_end_to_end() {
    let report = mev::validate_blog(voice_fixture_root()).expect("validate_blog");
    let hits = voice_tell_diags_for_file(&report.diagnostics, "exempt-blockquote.mdx");
    assert!(hits.is_empty(), "{hits:?}");
}

#[test]
fn phrase_in_frontmatter_is_exempt_end_to_end() {
    let report = mev::validate_blog(voice_fixture_root()).expect("validate_blog");
    let hits = voice_tell_diags_for_file(&report.diagnostics, "exempt-frontmatter.mdx");
    assert!(hits.is_empty(), "{hits:?}");
}

#[test]
fn clean_post_reports_no_voice_tells() {
    let report = mev::validate_blog(voice_fixture_root()).expect("validate_blog");
    let hits = voice_tell_diags_for_file(&report.diagnostics, "clean.mdx");
    assert!(hits.is_empty(), "{hits:?}");
}

// ---------------------------------------------------------------------------
// Whole-run sanity: every voice diagnostic in this fixture tree is a warning, and a corpus
// of nothing but voice tells is not a failing report.
// ---------------------------------------------------------------------------

#[test]
fn voice_fixture_tree_produces_only_warnings_and_is_not_a_failure() {
    let report = mev::validate_blog(voice_fixture_root()).expect("validate_blog");
    let voice_tells: Vec<&mev::Diagnostic> = report
        .diagnostics
        .iter()
        .filter(|d| d.locator == "W_VOICE_TELL")
        .collect();
    assert_eq!(voice_tells.len(), 3, "{:?}", report.diagnostics);
    assert!(voice_tells.iter().all(|d| d.severity == Severity::Warning));
}

// ---------------------------------------------------------------------------
// Live-corpus test — doubles as the operator report; skips cleanly on a fresh clone without
// the sibling learn-ai repo.
// ---------------------------------------------------------------------------

/// Ceiling on the number of `W_VOICE_TELL` findings the live corpus may produce before this
/// test fails. This is a bounded-count assertion, not an exact one, because posts get added
/// over time — a hardcoded post count would make this test brittle for reasons unrelated to
/// voice. At authoring time (2026-08-06) the seed list hit exactly 4 files (3 for
/// `production-ready`, 1 for `game-changing`, 0 for `actually bites you`) across the EN
/// published tree; 50 leaves generous room for corpus growth while still catching the failure
/// mode this test exists to catch — someone widening the seed list until the report becomes a
/// triage queue, which is exactly what the block's acceptance criterion rules out.
const LIVE_CORPUS_VOICE_TELL_CEILING: usize = 50;

#[test]
fn live_corpus_voice_tells_are_warning_only_and_bounded() {
    let live_root = Path::new("../../learn-ai/content/blog/published");
    if !live_root.exists() {
        eprintln!(
            "skipping: {} not present (fresh clone)",
            live_root.display()
        );
        return;
    }

    // (a) the run does not panic — `expect` here turns any panic-worthy state into a normal
    // test failure with a message, but the crawl + validate itself must not panic.
    let report = mev::validate_blog(live_root).expect("validate_blog must not panic or error");

    let voice_tells: Vec<&mev::Diagnostic> = report
        .diagnostics
        .iter()
        .filter(|d| d.locator == "W_VOICE_TELL")
        .collect();

    // (b) every voice-tripwire diagnostic is warning severity — a false positive here must
    // never be able to fail a push.
    assert!(
        voice_tells.iter().all(|d| d.severity == Severity::Warning),
        "every W_VOICE_TELL diagnostic must be Severity::Warning, got {voice_tells:?}"
    );

    // (c) the report is small enough to act on without triage — bounded, not exact, since
    // posts get added.
    assert!(
        voice_tells.len() <= LIVE_CORPUS_VOICE_TELL_CEILING,
        "expected at most {LIVE_CORPUS_VOICE_TELL_CEILING} W_VOICE_TELL findings over the live \
         corpus (operator-actionable-without-triage bar), got {}: {voice_tells:?}",
        voice_tells.len()
    );

    // Group findings by phrase -> files, and print them: this run doubles as the operator
    // report the block asks for.
    let mut by_phrase: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for diag in &voice_tells {
        // Message format: `voice tell "<phrase>" at line <n>` (see `voice::check_voice`).
        let phrase = diag
            .message
            .split_once('"')
            .and_then(|(_, rest)| rest.split_once('"'))
            .map(|(phrase, _)| phrase.to_string())
            .unwrap_or_else(|| diag.message.clone());
        by_phrase
            .entry(phrase)
            .or_default()
            .push(diag.file.display().to_string());
    }

    eprintln!("=== W_VOICE_TELL operator report (live corpus) ===");
    for (phrase, mut files) in by_phrase {
        files.sort();
        files.dedup();
        eprintln!("  \"{phrase}\" -> {} file(s): {files:?}", files.len());
    }
    eprintln!("=== end operator report ===");
}
