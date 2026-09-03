//! Fixture-backed integration tests for the funnel conformance checks (Phase 12, Block B,
//! Task 3): `E_FUNNEL_CTA_UNRESOLVED`, `E_FUNNEL_MISSING_UTM`, `E_FUNNEL_BARE_CAL_LINK`, and
//! `E_FUNNEL_RAW_ANALYTICS_ATTR`, wired through `BlogValidator`/`validate_blog` (this file's local helper over `mev_learn_ai::blog::BlogValidator`).
//!
//! Each code gets a passing and a failing fixture. The near-miss cases that make the checks
//! trustworthy — a `mailto:` bastiel address, a fully-tagged bastiel URL, a partially-tagged
//! one, and a resolvable vs. unresolvable `cta: module` target — each have their own fixture
//! under `tests/fixtures/funnel/content/`.
//!
//! **Re-measured 2026-08-06** (per the spec's amendment log, after `LA.21.C` retargeted 23
//! posts from `cta: booking` to `cta: bastiel`): running `mev validate --blog` over the live
//! `../../learn-ai/content/blog/published` tree today reports 0 errors and 17
//! `W_LINT_UNTAGGED_CODE_BLOCK`/`W_BLOG_PTBR_MISSING` warnings — **zero** `E_FUNNEL_*`
//! diagnostics of any kind. The live-corpus test below asserts exactly that and is the gate
//! that makes this block adoptable by learn-ai's `harness.json` — it is green on arrival.

use std::path::Path;

use mev_learn_ai::blog::BlogValidator;
use mev_learn_ai::{ContentValidator, Diagnostic, Report, Severity};

fn validate_blog(root: &Path) -> Report {
    BlogValidator::from_blog_root(root).run(root)
}

/// The funnel fixture tree's blog root — `<content>/blog/published`, matching the shape
/// `BlogValidator::from_blog_root` expects so it derives `learn_root` as
/// `<content>/learn` (a sibling directory, present in this fixture tree at
/// `tests/fixtures/funnel/content/learn/paths/demo/modules/01-intro.mdx`).
fn funnel_blog_root() -> &'static Path {
    Path::new("tests/fixtures/funnel/content/blog/published")
}

fn diags_for_file<'a>(diags: &'a [Diagnostic], file: &str) -> Vec<&'a Diagnostic> {
    diags.iter().filter(|d| d.file == Path::new(file)).collect()
}

fn codes_for_file<'a>(diags: &'a [Diagnostic], file: &str) -> Vec<&'a str> {
    diags_for_file(diags, file)
        .iter()
        .map(|d| d.locator.as_str())
        .collect()
}

// ---------------------------------------------------------------------------
// E_FUNNEL_CTA_UNRESOLVED
// ---------------------------------------------------------------------------

#[test]
fn unrecognized_cta_value_reports_e_funnel_cta_unresolved() {
    let report = validate_blog(funnel_blog_root());
    let hits = diags_for_file(&report.diagnostics, "cta-unresolved.mdx");
    let unresolved: Vec<_> = hits
        .iter()
        .filter(|d| d.locator == "E_FUNNEL_CTA_UNRESOLVED")
        .collect();
    assert_eq!(unresolved.len(), 1, "{:?}", report.diagnostics);
    assert_eq!(unresolved[0].severity, Severity::Error);
}

#[test]
fn cta_bastiel_resolves_clean_against_shipped_vocabulary() {
    let report = validate_blog(funnel_blog_root());
    let codes = codes_for_file(&report.diagnostics, "cta-bastiel-clean.mdx");
    assert!(!codes.contains(&"E_FUNNEL_CTA_UNRESOLVED"), "{codes:?}");
}

#[test]
fn post_with_no_cta_key_reports_no_diagnostic() {
    let report = validate_blog(funnel_blog_root());
    let codes = codes_for_file(&report.diagnostics, "cta-no-key.mdx");
    assert!(
        !codes.contains(&"E_FUNNEL_CTA_UNRESOLVED"),
        "a post with no cta key must not be flagged, got {codes:?}"
    );
}

#[test]
fn cta_module_with_resolvable_target_reports_no_diagnostic() {
    let report = validate_blog(funnel_blog_root());
    let codes = codes_for_file(&report.diagnostics, "cta-module-resolves.mdx");
    assert!(
        !codes.contains(&"E_FUNNEL_CTA_UNRESOLVED"),
        "ctaTarget `demo/intro` resolves against the fixture learn tree, got {codes:?}"
    );
}

#[test]
fn cta_module_with_unresolvable_target_reports_e_funnel_cta_unresolved() {
    let report = validate_blog(funnel_blog_root());
    let hits = diags_for_file(&report.diagnostics, "cta-module-unresolved.mdx");
    let unresolved: Vec<_> = hits
        .iter()
        .filter(|d| d.locator == "E_FUNNEL_CTA_UNRESOLVED")
        .collect();
    assert_eq!(unresolved.len(), 1, "{:?}", report.diagnostics);
    assert!(unresolved[0].message.contains("demo/does-not-exist"));
}

// ---------------------------------------------------------------------------
// E_FUNNEL_MISSING_UTM
// ---------------------------------------------------------------------------

#[test]
fn bastiel_link_with_no_utm_params_reports_e_funnel_missing_utm() {
    let report = validate_blog(funnel_blog_root());
    let hits = diags_for_file(&report.diagnostics, "utm-missing.mdx");
    let missing: Vec<_> = hits
        .iter()
        .filter(|d| d.locator == "E_FUNNEL_MISSING_UTM")
        .collect();
    assert_eq!(missing.len(), 1, "{:?}", report.diagnostics);
    assert_eq!(missing[0].severity, Severity::Error);
}

#[test]
fn bastiel_link_with_all_three_utm_params_reports_no_diagnostic() {
    let report = validate_blog(funnel_blog_root());
    let codes = codes_for_file(&report.diagnostics, "utm-full.mdx");
    assert!(!codes.contains(&"E_FUNNEL_MISSING_UTM"), "{codes:?}");
}

#[test]
fn bastiel_link_with_two_of_three_utm_params_reports_e_funnel_missing_utm() {
    let report = validate_blog(funnel_blog_root());
    let hits = diags_for_file(&report.diagnostics, "utm-partial.mdx");
    let missing: Vec<_> = hits
        .iter()
        .filter(|d| d.locator == "E_FUNNEL_MISSING_UTM")
        .collect();
    assert_eq!(missing.len(), 1, "{:?}", report.diagnostics);
    assert!(
        missing[0].message.contains("utm_campaign"),
        "expected the missing param named in the message: {:?}",
        missing[0].message
    );
}

#[test]
fn mailto_bastiel_reference_reports_no_diagnostic() {
    let report = validate_blog(funnel_blog_root());
    let codes = codes_for_file(&report.diagnostics, "mailto-clean.mdx");
    assert!(
        !codes.contains(&"E_FUNNEL_MISSING_UTM"),
        "mailto: references must never be flagged, got {codes:?}"
    );
}

// ---------------------------------------------------------------------------
// E_FUNNEL_BARE_CAL_LINK
// ---------------------------------------------------------------------------

#[test]
fn cal_com_link_reports_e_funnel_bare_cal_link() {
    let report = validate_blog(funnel_blog_root());
    let hits = diags_for_file(&report.diagnostics, "cal-link.mdx");
    let bare: Vec<_> = hits
        .iter()
        .filter(|d| d.locator == "E_FUNNEL_BARE_CAL_LINK")
        .collect();
    assert_eq!(bare.len(), 1, "{:?}", report.diagnostics);
    assert_eq!(bare[0].severity, Severity::Error);
}

#[test]
fn post_without_cal_link_reports_no_diagnostic() {
    let report = validate_blog(funnel_blog_root());
    let codes = codes_for_file(&report.diagnostics, "clean.mdx");
    assert!(!codes.contains(&"E_FUNNEL_BARE_CAL_LINK"), "{codes:?}");
}

// ---------------------------------------------------------------------------
// E_FUNNEL_RAW_ANALYTICS_ATTR
// ---------------------------------------------------------------------------

#[test]
fn raw_data_umami_attr_reports_e_funnel_raw_analytics_attr() {
    let report = validate_blog(funnel_blog_root());
    let hits = diags_for_file(&report.diagnostics, "analytics-attr.mdx");
    let raw: Vec<_> = hits
        .iter()
        .filter(|d| d.locator == "E_FUNNEL_RAW_ANALYTICS_ATTR")
        .collect();
    assert_eq!(raw.len(), 1, "{:?}", report.diagnostics);
    assert_eq!(raw[0].severity, Severity::Error);
}

#[test]
fn post_without_raw_analytics_attr_reports_no_diagnostic() {
    let report = validate_blog(funnel_blog_root());
    let codes = codes_for_file(&report.diagnostics, "clean.mdx");
    assert!(!codes.contains(&"E_FUNNEL_RAW_ANALYTICS_ATTR"), "{codes:?}");
}

// ---------------------------------------------------------------------------
// The fully-clean fixture post never trips any of the four funnel codes.
// ---------------------------------------------------------------------------

#[test]
fn clean_post_reports_no_funnel_diagnostics_at_all() {
    let report = validate_blog(funnel_blog_root());
    let codes = codes_for_file(&report.diagnostics, "clean.mdx");
    let funnel_codes: Vec<_> = codes
        .iter()
        .filter(|c| c.starts_with("E_FUNNEL_"))
        .collect();
    assert!(funnel_codes.is_empty(), "{funnel_codes:?}");
}

// ---------------------------------------------------------------------------
// Live-corpus zero-violation assertion — the gate that makes this block adoptable by
// learn-ai's harness.json. Skips cleanly when the sibling repo is absent (fresh clone).
// ---------------------------------------------------------------------------

#[test]
fn live_blog_corpus_reports_zero_e_funnel_diagnostics() {
    let live_root = Path::new("../../../../learn-ai/content/blog/published");
    if !live_root.exists() {
        eprintln!(
            "skipping: {} not present (fresh clone)",
            live_root.display()
        );
        return;
    }

    let report = validate_blog(live_root);
    let funnel_hits: Vec<&Diagnostic> = report
        .diagnostics
        .iter()
        .filter(|d| d.locator.starts_with("E_FUNNEL_"))
        .collect();
    assert!(
        funnel_hits.is_empty(),
        "expected zero E_FUNNEL_* diagnostics over the live blog tree, got {funnel_hits:?}"
    );
}
