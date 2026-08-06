//! Fixture-backed integration tests for `BlogValidator` (Phase 12, Block A, Task 5).
//!
//! Covers all six diagnostic codes the block introduces, each with a passing and a failing
//! case, plus:
//! - a regression pin proving `mev::validate` (lint off) stays behaviourally unchanged even
//!   over a module that would trip the shared lint passes if they ran, and
//! - a live-tree smoke test over the real learn-ai blog tree that skips cleanly when absent.

use std::path::Path;

use mev::{BlogValidator, ContentValidator, Locale, Severity};

/// The blog fixture tree checked into `tests/fixtures/blog/`.
fn blog_fixture_root() -> &'static Path {
    Path::new("tests/fixtures/blog")
}

/// The learn-tree fixture used for the byte-identical regression pin.
fn learn_tree_fixture_root() -> &'static Path {
    Path::new("tests/fixtures/blog-learn-tree")
}

fn diags_for_file<'a>(diags: &'a [mev::Diagnostic], file: &str) -> Vec<&'a mev::Diagnostic> {
    diags.iter().filter(|d| d.file == Path::new(file)).collect()
}

// ---------------------------------------------------------------------------
// E_BLOG_MISSING_FIELD
// ---------------------------------------------------------------------------

#[test]
fn missing_title_reports_e_blog_missing_field() {
    let report = mev::validate_blog(blog_fixture_root()).expect("validate_blog");
    let hits = diags_for_file(&report.diagnostics, "missing-title.mdx");
    let missing = hits
        .iter()
        .filter(|d| d.locator == "E_BLOG_MISSING_FIELD")
        .collect::<Vec<_>>();
    assert_eq!(missing.len(), 1, "{:?}", report.diagnostics);
    assert_eq!(missing[0].severity, Severity::Error);
    assert!(missing[0].message.contains("title"));
}

#[test]
fn missing_date_reports_e_blog_missing_field() {
    let report = mev::validate_blog(blog_fixture_root()).expect("validate_blog");
    let hits = diags_for_file(&report.diagnostics, "missing-date.mdx");
    let missing = hits
        .iter()
        .filter(|d| d.locator == "E_BLOG_MISSING_FIELD")
        .collect::<Vec<_>>();
    assert_eq!(missing.len(), 1, "{:?}", report.diagnostics);
    assert!(missing[0].message.contains("date"));
}

#[test]
fn missing_excerpt_reports_e_blog_missing_field() {
    let report = mev::validate_blog(blog_fixture_root()).expect("validate_blog");
    let hits = diags_for_file(&report.diagnostics, "missing-excerpt.mdx");
    let missing = hits
        .iter()
        .filter(|d| d.locator == "E_BLOG_MISSING_FIELD")
        .collect::<Vec<_>>();
    assert_eq!(missing.len(), 1, "{:?}", report.diagnostics);
    assert!(missing[0].message.contains("excerpt"));
}

#[test]
fn complete_frontmatter_post_reports_no_missing_field() {
    let report = mev::validate_blog(blog_fixture_root()).expect("validate_blog");
    let hits = diags_for_file(&report.diagnostics, "clean.mdx");
    assert!(
        hits.iter().all(|d| d.locator != "E_BLOG_MISSING_FIELD"),
        "{hits:?}"
    );
}

// ---------------------------------------------------------------------------
// E_BLOG_MALFORMED_FRONTMATTER
// ---------------------------------------------------------------------------

#[test]
fn no_frontmatter_reports_malformed_frontmatter() {
    let report = mev::validate_blog(blog_fixture_root()).expect("validate_blog");
    let hits = diags_for_file(&report.diagnostics, "no-frontmatter.mdx");
    let malformed = hits
        .iter()
        .filter(|d| d.locator == "E_BLOG_MALFORMED_FRONTMATTER")
        .collect::<Vec<_>>();
    assert_eq!(malformed.len(), 1, "{hits:?}");
    assert_eq!(malformed[0].severity, Severity::Error);
}

#[test]
fn malformed_yaml_reports_malformed_frontmatter() {
    let report = mev::validate_blog(blog_fixture_root()).expect("validate_blog");
    let hits = diags_for_file(&report.diagnostics, "malformed-yaml.mdx");
    let malformed = hits
        .iter()
        .filter(|d| d.locator == "E_BLOG_MALFORMED_FRONTMATTER")
        .collect::<Vec<_>>();
    assert_eq!(malformed.len(), 1, "{hits:?}");
}

#[test]
fn well_formed_frontmatter_post_reports_no_malformed_frontmatter() {
    let report = mev::validate_blog(blog_fixture_root()).expect("validate_blog");
    let hits = diags_for_file(&report.diagnostics, "clean.mdx");
    assert!(
        hits.iter()
            .all(|d| d.locator != "E_BLOG_MALFORMED_FRONTMATTER"),
        "{hits:?}"
    );
}

// ---------------------------------------------------------------------------
// W_BLOG_PTBR_MISSING
// ---------------------------------------------------------------------------

#[test]
fn en_post_without_ptbr_counterpart_reports_parity_warning() {
    let report = mev::validate_blog(blog_fixture_root()).expect("validate_blog");
    let hits = diags_for_file(&report.diagnostics, "no-ptbr.mdx");
    let parity = hits
        .iter()
        .filter(|d| d.locator == "W_BLOG_PTBR_MISSING")
        .collect::<Vec<_>>();
    assert_eq!(parity.len(), 1, "{:?}", report.diagnostics);
    assert_eq!(parity[0].severity, Severity::Warning);
}

#[test]
fn en_post_with_ptbr_counterpart_reports_no_parity_warning() {
    let report = mev::validate_blog(blog_fixture_root()).expect("validate_blog");
    let hits = diags_for_file(&report.diagnostics, "clean.mdx");
    assert!(
        hits.iter().all(|d| d.locator != "W_BLOG_PTBR_MISSING"),
        "{hits:?}"
    );
}

// ---------------------------------------------------------------------------
// W_LINT_UNTAGGED_CODE_BLOCK
// ---------------------------------------------------------------------------

#[test]
fn untagged_fence_reports_lint_warning() {
    let report = mev::validate_blog(blog_fixture_root()).expect("validate_blog");
    let hits = diags_for_file(&report.diagnostics, "untagged-fence.mdx");
    let lint = hits
        .iter()
        .filter(|d| d.locator == "W_LINT_UNTAGGED_CODE_BLOCK")
        .collect::<Vec<_>>();
    assert_eq!(lint.len(), 1, "{:?}", report.diagnostics);
    assert_eq!(lint[0].severity, Severity::Warning);
}

#[test]
fn tagged_fence_post_reports_no_untagged_code_block_warning() {
    let report = mev::validate_blog(blog_fixture_root()).expect("validate_blog");
    let hits = diags_for_file(&report.diagnostics, "clean.mdx");
    assert!(
        hits.iter()
            .all(|d| d.locator != "W_LINT_UNTAGGED_CODE_BLOCK"),
        "{hits:?}"
    );
}

// ---------------------------------------------------------------------------
// E_LINT_DEAD_LOCAL_LINK / E_LINT_DEAD_ASSET
// ---------------------------------------------------------------------------

#[test]
fn dead_relative_link_reports_dead_local_link() {
    let report = mev::validate_blog(blog_fixture_root()).expect("validate_blog");
    let hits = diags_for_file(&report.diagnostics, "dead-links.mdx");
    let dead_link = hits
        .iter()
        .filter(|d| d.locator == "E_LINT_DEAD_LOCAL_LINK")
        .collect::<Vec<_>>();
    assert_eq!(dead_link.len(), 1, "{:?}", report.diagnostics);
    assert_eq!(dead_link[0].severity, Severity::Error);
}

#[test]
fn dead_image_reference_reports_dead_asset() {
    let report = mev::validate_blog(blog_fixture_root()).expect("validate_blog");
    let hits = diags_for_file(&report.diagnostics, "dead-links.mdx");
    let dead_asset = hits
        .iter()
        .filter(|d| d.locator == "E_LINT_DEAD_ASSET")
        .collect::<Vec<_>>();
    assert_eq!(dead_asset.len(), 1, "{:?}", report.diagnostics);
    assert_eq!(dead_asset[0].severity, Severity::Error);
}

#[test]
fn absolute_url_and_anchor_only_links_report_no_dead_link_diagnostics() {
    let report = mev::validate_blog(blog_fixture_root()).expect("validate_blog");
    let hits = diags_for_file(&report.diagnostics, "clean-links.mdx");
    assert!(
        hits.iter()
            .all(|d| d.locator != "E_LINT_DEAD_LOCAL_LINK" && d.locator != "E_LINT_DEAD_ASSET"),
        "{hits:?}"
    );
}

// ---------------------------------------------------------------------------
// Whole-run sanity: every published post is crawled
// ---------------------------------------------------------------------------

#[test]
fn every_fixture_post_is_crawled_and_classified() {
    let (posts, _diags) = BlogValidator.crawl(blog_fixture_root());
    let en_slugs: Vec<&str> = posts
        .iter()
        .filter(|p| p.locale == Locale::En)
        .map(|p| p.slug.as_str())
        .collect();
    for expected in [
        "clean",
        "missing-title",
        "missing-date",
        "missing-excerpt",
        "no-frontmatter",
        "malformed-yaml",
        "untagged-fence",
        "dead-links",
        "clean-links",
        "no-ptbr",
    ] {
        assert!(
            en_slugs.contains(&expected),
            "expected {expected} in crawled EN slugs, got {en_slugs:?}"
        );
    }

    let ptbr_slugs: Vec<&str> = posts
        .iter()
        .filter(|p| p.locale == Locale::PtBr)
        .map(|p| p.slug.as_str())
        .collect();
    assert!(ptbr_slugs.contains(&"clean"));
}

// ---------------------------------------------------------------------------
// Regression pin: `mev::validate` stays byte-identical (lint always off)
// ---------------------------------------------------------------------------

#[test]
fn validate_regression_pin_on_learn_tree_fixture() {
    // The fixture module carries an untagged fence and a dead relative link on purpose. If
    // `mev::validate` ever started running the shared lint passes by default, this test would
    // start failing — that is exactly the behaviour this pin exists to catch.
    let report = mev::validate(learn_tree_fixture_root()).expect("validate");
    assert!(
        report.diagnostics.is_empty(),
        "mev::validate must stay lint-off and report nothing on a structurally-valid module, \
         got: {:?}",
        report.diagnostics
    );
}

#[test]
fn validate_with_lint_reports_lint_diagnostics_where_validate_does_not() {
    let plain = mev::validate(learn_tree_fixture_root()).expect("validate");
    let linted = mev::validate_with_lint(learn_tree_fixture_root()).expect("validate_with_lint");

    assert!(plain.diagnostics.is_empty(), "{:?}", plain.diagnostics);
    assert!(
        linted
            .diagnostics
            .iter()
            .any(|d| d.locator == "W_LINT_UNTAGGED_CODE_BLOCK"),
        "{:?}",
        linted.diagnostics
    );
    assert!(
        linted
            .diagnostics
            .iter()
            .any(|d| d.locator == "E_LINT_DEAD_LOCAL_LINK"),
        "{:?}",
        linted.diagnostics
    );
}

// ---------------------------------------------------------------------------
// Live-tree smoke test — skips cleanly on a fresh clone without the sibling repo
// ---------------------------------------------------------------------------

#[test]
fn validate_blog_does_not_panic_over_the_live_blog_tree() {
    let live_root = Path::new("../../learn-ai/content/blog/published");
    if !live_root.exists() {
        eprintln!(
            "skipping: {} not present (fresh clone)",
            live_root.display()
        );
        return;
    }

    let report = mev::validate_blog(live_root).expect("validate_blog must not panic or error");
    // No assertion on diagnostic content — the live tree's content is out of this test's
    // control. The point is that the crawl + validate pass over every real file without
    // panicking and returns a `Report`.
    let _ = report.diagnostics.len();
}
