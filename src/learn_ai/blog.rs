//! Blog content validator (Phase 12, Block A, Task 2).
//!
//! The third [`ContentValidator`] impl, alongside `LearnAiValidator` (learn modules) and
//! `BrainValidator` (OKF). Crawls the learn-ai blog tree — `published/*.mdx` (EN) and
//! `published/pt-BR/*.mdx` (pt-BR) — checks required frontmatter, EN → pt-BR filename parity,
//! and runs the shared lint passes from `lint.rs` over every post.
//!
//! Blog crawl classification is intentionally simpler than the learn tree's: it reuses
//! [`Locale`] rather than inventing a second locale enum, and does not build a `Corpus` —
//! a flat `Vec<BlogPost>` is enough for this tree's shape.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::crawl::Locale;
use super::lint;
use crate::Diagnostic;
use crate::Report;
use crate::shared::{extract_frontmatter, non_empty};
use crate::validator::ContentValidator;

/// A single classified blog post.
#[derive(Debug, Clone)]
pub struct BlogPost {
    /// Absolute path as walked (joined on the blog root).
    pub path: PathBuf,
    /// Path relative to the blog root — used for diagnostic locators and display.
    pub rel: PathBuf,
    /// File stem (e.g. `my-post` for `my-post.mdx`).
    pub slug: String,
    /// Locale this post belongs to.
    pub locale: Locale,
}

/// Frontmatter fields this block requires, plus the `cta` / `ctaTarget` fields `MV.12.B`'s
/// funnel checks need. Unknown keys are tolerated (no `deny_unknown_fields`).
#[derive(Debug, Deserialize, Default)]
struct BlogFrontmatter {
    title: Option<String>,
    date: Option<String>,
    excerpt: Option<String>,
    cta: Option<String>,
    #[serde(rename = "ctaTarget")]
    cta_target: Option<String>,
}

/// Walk `root`, classify every `.mdx` file as EN (direct children) or pt-BR (`pt-BR/`
/// children), and return the posts plus any crawl-time diagnostics.
///
/// Non-`.mdx` files (`CLAUDE.md`, `GEMINI.md`, `queue.json`, …) are skipped silently. Order is
/// deterministic: EN posts sorted by path, then pt-BR posts sorted by path. The pt-BR parity
/// check (`W_BLOG_PTBR_MISSING`) is a corpus-level check and is emitted here, not from
/// `validate_item` — it is a property of the EN/pt-BR pairing, not of a single post.
pub fn crawl(root: &Path) -> (Vec<BlogPost>, Vec<Diagnostic>) {
    let mut posts = Vec::new();
    let mut diags = Vec::new();

    let ptbr_root = root.join("pt-BR");

    let en_entries = list_mdx_files(root);
    let ptbr_entries = list_mdx_files(&ptbr_root);

    let ptbr_slugs: HashSet<String> = ptbr_entries.iter().map(|(slug, _)| slug.clone()).collect();

    for (slug, path) in &en_entries {
        let rel = path.strip_prefix(root).unwrap_or(path).to_path_buf();
        if !ptbr_slugs.contains(slug) {
            diags.push(Diagnostic::warning(
                rel.clone(),
                "W_BLOG_PTBR_MISSING",
                format!("EN post `{slug}` has no pt-BR counterpart"),
            ));
        }
        posts.push(BlogPost {
            path: path.clone(),
            rel,
            slug: slug.clone(),
            locale: Locale::En,
        });
    }

    for (slug, path) in &ptbr_entries {
        let rel = path.strip_prefix(root).unwrap_or(path).to_path_buf();
        posts.push(BlogPost {
            path: path.clone(),
            rel,
            slug: slug.clone(),
            locale: Locale::PtBr,
        });
    }

    (posts, diags)
}

/// List `.mdx` files directly under `dir` (non-recursive), returning `(slug, path)` pairs
/// sorted by path for a deterministic crawl order. A missing or unreadable `dir` yields an
/// empty list rather than an error — an absent `pt-BR/` subdirectory is normal, not a defect.
fn list_mdx_files(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut entries: Vec<(String, PathBuf)> = match std::fs::read_dir(dir) {
        Ok(read) => read
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.is_file() && p.extension().and_then(|e| e.to_str()) == Some("mdx"))
            .filter_map(|p| {
                let slug = p.file_stem()?.to_str()?.to_string();
                Some((slug, p))
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    entries.sort_by(|a, b| a.1.cmp(&b.1));
    entries
}

/// Validate a single post's frontmatter, run the shared lint passes over it, then the
/// `MV.12.B` funnel checks.
///
/// A read failure short-circuits to a single `E_BLOG_MALFORMED_FRONTMATTER` diagnostic; the
/// lint and funnel passes never run over content that could not be read.
///
/// `content_root` is threaded through to [`lint::lint_local_links`] for route-aware
/// resolution of site-absolute links (Task 8). `learn_root` is threaded through to
/// [`funnel::check_cta`] for filesystem resolution of `cta: module` targets (Task 2). Both
/// `None` skip the corresponding resolution rather than guessing — callers that only have a
/// bare `&BlogPost` (e.g. `ContentValidator::validate_item`, used directly by tests) get that
/// conservative behaviour; [`BlogValidator::run`] derives/carries the real roots and passes
/// them through.
///
/// The `cta`/`ctaTarget` frontmatter fields are parsed once here (by [`frontmatter_diagnostics`])
/// and passed through to the funnel check rather than re-parsed — the funnel checks that scan
/// raw `source` (`check_utm`/`check_cal_link`/`check_analytics_attr`) still run even when the
/// frontmatter itself is malformed, since they do not depend on it.
fn validate_post(
    post: &BlogPost,
    content_root: Option<&Path>,
    learn_root: Option<&Path>,
) -> Vec<Diagnostic> {
    let contents = match std::fs::read_to_string(&post.path) {
        Ok(c) => c,
        Err(e) => {
            return vec![Diagnostic::error(
                post.rel.clone(),
                "E_BLOG_MALFORMED_FRONTMATTER",
                format!("could not read file: {e}"),
            )];
        }
    };

    let (mut diags, fm) = frontmatter_diagnostics(post, &contents);
    diags.extend(lint::lint_code_blocks(&post.rel, &contents));
    diags.extend(lint::lint_local_links(
        &post.path,
        &post.rel,
        &contents,
        content_root,
    ));

    let funnel_fm = super::funnel::Frontmatter {
        cta: fm.as_ref().and_then(|f| f.cta.clone()),
        cta_target: fm.as_ref().and_then(|f| f.cta_target.clone()),
    };
    diags.extend(super::funnel::check_cta(&post.rel, &funnel_fm, learn_root));
    diags.extend(super::funnel::check_utm(&post.rel, &contents));
    diags.extend(super::funnel::check_cal_link(&post.rel, &contents));
    diags.extend(super::funnel::check_analytics_attr(&post.rel, &contents));

    diags
}

/// Parse and validate the leading `---` YAML frontmatter block: absent/unparseable ->
/// `E_BLOG_MALFORMED_FRONTMATTER`, returning `(diags, None)`; each missing/empty required field
/// (`title`, `date`, `excerpt`) -> one `E_BLOG_MISSING_FIELD`. On success, returns the parsed
/// [`BlogFrontmatter`] alongside any field diagnostics so callers (the funnel checks) can reuse
/// it rather than re-parsing the file.
fn frontmatter_diagnostics(
    post: &BlogPost,
    contents: &str,
) -> (Vec<Diagnostic>, Option<BlogFrontmatter>) {
    let yaml = match extract_frontmatter(contents) {
        Some(y) => y,
        None => {
            return (
                vec![Diagnostic::error(
                    post.rel.clone(),
                    "E_BLOG_MALFORMED_FRONTMATTER",
                    "missing or unterminated frontmatter block (expected leading --- fence)",
                )],
                None,
            );
        }
    };

    let fm: BlogFrontmatter = match serde_yaml::from_str(yaml) {
        Ok(f) => f,
        Err(e) => {
            return (
                vec![Diagnostic::error(
                    post.rel.clone(),
                    "E_BLOG_MALFORMED_FRONTMATTER",
                    format!("malformed YAML in frontmatter: {e}"),
                )],
                None,
            );
        }
    };

    let mut diags = Vec::new();
    require_field(post, "title", &fm.title, &mut diags);
    require_field(post, "date", &fm.date, &mut diags);
    require_field(post, "excerpt", &fm.excerpt, &mut diags);
    (diags, Some(fm))
}

/// Push an `E_BLOG_MISSING_FIELD` diagnostic naming `field_name` when `value` is absent or
/// blank.
fn require_field(
    post: &BlogPost,
    field_name: &str,
    value: &Option<String>,
    diags: &mut Vec<Diagnostic>,
) {
    if non_empty(value).is_none() {
        diags.push(Diagnostic::error(
            post.rel.clone(),
            "E_BLOG_MISSING_FIELD",
            format!("missing required frontmatter field `{field_name}`"),
        ));
    }
}

/// The `ContentValidator` for the learn-ai blog tree.
///
/// `learn_root`, when `Some`, roots the on-disk existence half of `funnel::check_cta`'s
/// `cta: module` handling (`MV.12.B` Task 2). It is resolved once at construction time — see
/// [`BlogValidator::from_blog_root`] — rather than re-derived per item.
#[derive(Debug, Default, Clone)]
pub struct BlogValidator {
    pub learn_root: Option<PathBuf>,
}

impl BlogValidator {
    /// Construct a validator with an explicit `learn_root` (or `None` to skip module-existence
    /// resolution entirely) — the constructor tests use to point it anywhere.
    pub fn new(learn_root: Option<PathBuf>) -> Self {
        Self { learn_root }
    }

    /// Derive `learn_root` from a blog root, per the block's `<content>/blog/published` ->
    /// `<content>/learn` mapping (reusing [`lint::derive_content_root`]), and keep it only when
    /// that directory actually exists on disk — a partial checkout (no sibling learn tree)
    /// degrades to `None` rather than pointing at a directory that isn't there.
    pub fn from_blog_root(blog_root: &Path) -> Self {
        let learn_root = lint::derive_content_root(blog_root)
            .map(|content_root| content_root.join("learn"))
            .filter(|p| p.is_dir());
        Self { learn_root }
    }
}

impl ContentValidator for BlogValidator {
    type Item = BlogPost;

    fn crawl(&self, root: &Path) -> (Vec<BlogPost>, Vec<Diagnostic>) {
        crawl(root)
    }

    /// Used directly (e.g. by tests calling `validate_item` on a bare `BlogPost`) without a
    /// known content root — conservatively skips route resolution for any absolute link
    /// rather than guessing. [`run`](ContentValidator::run) is overridden below to derive and
    /// thread through the real content root for the actual `mev validate --blog` path.
    fn validate_item(&self, item: &BlogPost) -> Vec<Diagnostic> {
        validate_post(item, None, self.learn_root.as_deref())
    }

    /// Overridden (not the default driver) so every item's lint pass gets route-aware
    /// resolution: the content root is derived once from `root` here and threaded through to
    /// [`validate_post`], since `ContentValidator::validate_item`'s signature has no root
    /// parameter to carry it (Task 8). `learn_root` was already resolved at construction time
    /// and is carried through unchanged.
    fn run(&self, root: &Path) -> Report {
        let content_root = lint::derive_content_root(root);
        let (items, mut diagnostics) = self.crawl(root);
        for item in &items {
            diagnostics.extend(validate_post(
                item,
                content_root.as_deref(),
                self.learn_root.as_deref(),
            ));
        }
        Report { diagnostics }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Severity;

    /// Build a blog root fixture: `root/*.mdx` (EN) and optionally `root/pt-BR/*.mdx`.
    /// Returns the root dir; caller is responsible for cleanup.
    fn fixture_root(prefix: &str) -> PathBuf {
        let dir = crate::testsupport::unique_temp_dir(prefix);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_post(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        path
    }

    fn clean_frontmatter() -> &'static str {
        "---\ntitle: My Post\ndate: 2026-01-01\nexcerpt: A short summary.\n---\nBody text.\n"
    }

    fn diags_with_locator<'a>(diags: &'a [Diagnostic], locator: &str) -> Vec<&'a Diagnostic> {
        diags.iter().filter(|d| d.locator == locator).collect()
    }

    // -----------------------------------------------------------------
    // Frontmatter: clean + each required field missing
    // -----------------------------------------------------------------

    #[test]
    fn complete_frontmatter_is_clean() {
        let dir = fixture_root("mev-blog-clean");
        let path = write_post(&dir, "post.mdx", clean_frontmatter());
        let post = BlogPost {
            path: path.clone(),
            rel: PathBuf::from("post.mdx"),
            slug: "post".to_string(),
            locale: Locale::En,
        };
        let diags = validate_post(&post, None, None);
        assert!(diags.is_empty(), "expected clean post, got {diags:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_title_reports_missing_field() {
        let dir = fixture_root("mev-blog-missing-title");
        let body = "---\ndate: 2026-01-01\nexcerpt: A short summary.\n---\nBody.\n";
        let path = write_post(&dir, "post.mdx", body);
        let post = BlogPost {
            path,
            rel: PathBuf::from("post.mdx"),
            slug: "post".to_string(),
            locale: Locale::En,
        };
        let diags = validate_post(&post, None, None);
        let missing = diags_with_locator(&diags, "E_BLOG_MISSING_FIELD");
        assert_eq!(missing.len(), 1, "{diags:?}");
        assert!(missing[0].message.contains("title"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_date_reports_missing_field() {
        let dir = fixture_root("mev-blog-missing-date");
        let body = "---\ntitle: My Post\nexcerpt: A short summary.\n---\nBody.\n";
        let path = write_post(&dir, "post.mdx", body);
        let post = BlogPost {
            path,
            rel: PathBuf::from("post.mdx"),
            slug: "post".to_string(),
            locale: Locale::En,
        };
        let diags = validate_post(&post, None, None);
        let missing = diags_with_locator(&diags, "E_BLOG_MISSING_FIELD");
        assert_eq!(missing.len(), 1, "{diags:?}");
        assert!(missing[0].message.contains("date"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_excerpt_reports_missing_field() {
        let dir = fixture_root("mev-blog-missing-excerpt");
        let body = "---\ntitle: My Post\ndate: 2026-01-01\n---\nBody.\n";
        let path = write_post(&dir, "post.mdx", body);
        let post = BlogPost {
            path,
            rel: PathBuf::from("post.mdx"),
            slug: "post".to_string(),
            locale: Locale::En,
        };
        let diags = validate_post(&post, None, None);
        let missing = diags_with_locator(&diags, "E_BLOG_MISSING_FIELD");
        assert_eq!(missing.len(), 1, "{diags:?}");
        assert!(missing[0].message.contains("excerpt"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn empty_string_field_counts_as_missing() {
        let dir = fixture_root("mev-blog-empty-field");
        let body = "---\ntitle: \"\"\ndate: 2026-01-01\nexcerpt: A short summary.\n---\nBody.\n";
        let path = write_post(&dir, "post.mdx", body);
        let post = BlogPost {
            path,
            rel: PathBuf::from("post.mdx"),
            slug: "post".to_string(),
            locale: Locale::En,
        };
        let diags = validate_post(&post, None, None);
        let missing = diags_with_locator(&diags, "E_BLOG_MISSING_FIELD");
        assert_eq!(missing.len(), 1, "{diags:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn no_frontmatter_at_all_reports_malformed() {
        let dir = fixture_root("mev-blog-no-frontmatter");
        let path = write_post(&dir, "post.mdx", "Just body text, no frontmatter.\n");
        let post = BlogPost {
            path,
            rel: PathBuf::from("post.mdx"),
            slug: "post".to_string(),
            locale: Locale::En,
        };
        let diags = validate_post(&post, None, None);
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].locator, "E_BLOG_MALFORMED_FRONTMATTER");
        assert_eq!(diags[0].severity, Severity::Error);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn malformed_yaml_reports_malformed() {
        let dir = fixture_root("mev-blog-malformed-yaml");
        // Unterminated flow mapping -> invalid YAML.
        let body = "---\ntitle: [unterminated\ndate: 2026-01-01\n---\nBody.\n";
        let path = write_post(&dir, "post.mdx", body);
        let post = BlogPost {
            path,
            rel: PathBuf::from("post.mdx"),
            slug: "post".to_string(),
            locale: Locale::En,
        };
        let diags = validate_post(&post, None, None);
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].locator, "E_BLOG_MALFORMED_FRONTMATTER");
        std::fs::remove_dir_all(&dir).ok();
    }

    // -----------------------------------------------------------------
    // Lint passes wired through validate_item
    // -----------------------------------------------------------------

    #[test]
    fn validate_item_runs_lint_passes() {
        let dir = fixture_root("mev-blog-lint-wired");
        let mut body = clean_frontmatter().to_string();
        body.push_str("\n```\nuntagged code\n```\n");
        let path = write_post(&dir, "post.mdx", &body);
        let post = BlogPost {
            path,
            rel: PathBuf::from("post.mdx"),
            slug: "post".to_string(),
            locale: Locale::En,
        };
        let diags = validate_post(&post, None, None);
        assert!(
            diags
                .iter()
                .any(|d| d.locator == "W_LINT_UNTAGGED_CODE_BLOCK"),
            "{diags:?}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    // -----------------------------------------------------------------
    // crawl: pt-BR parity, locale classification, non-.mdx skip
    // -----------------------------------------------------------------

    #[test]
    fn en_post_with_ptbr_counterpart_has_no_parity_warning() {
        let dir = fixture_root("mev-blog-parity-present");
        std::fs::create_dir_all(dir.join("pt-BR")).unwrap();
        write_post(&dir, "hello.mdx", clean_frontmatter());
        write_post(&dir.join("pt-BR"), "hello.mdx", clean_frontmatter());

        let (posts, diags) = crawl(&dir);
        assert_eq!(posts.len(), 2, "{posts:?}");
        assert!(
            diags.is_empty(),
            "counterpart present, expected no parity warning: {diags:?}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn en_post_without_ptbr_counterpart_gets_exactly_one_warning() {
        let dir = fixture_root("mev-blog-parity-missing");
        write_post(&dir, "solo.mdx", clean_frontmatter());

        let (posts, diags) = crawl(&dir);
        assert_eq!(posts.len(), 1);
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].locator, "W_BLOG_PTBR_MISSING");
        assert_eq!(diags[0].severity, Severity::Warning);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ptbr_only_post_does_not_trigger_reverse_parity_warning() {
        let dir = fixture_root("mev-blog-ptbr-only");
        std::fs::create_dir_all(dir.join("pt-BR")).unwrap();
        write_post(&dir.join("pt-BR"), "somente.mdx", clean_frontmatter());

        let (posts, diags) = crawl(&dir);
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].locale, Locale::PtBr);
        assert!(
            diags.is_empty(),
            "pt-BR-only file must not trigger a reverse-parity warning: {diags:?}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn non_mdx_files_are_ignored_by_crawl() {
        let dir = fixture_root("mev-blog-non-mdx");
        write_post(&dir, "post.mdx", clean_frontmatter());
        write_post(&dir, "CLAUDE.md", "not a post");
        write_post(&dir, "queue.json", "{}");

        let (posts, diags) = crawl(&dir);
        assert_eq!(posts.len(), 1, "{posts:?}");
        // The lone post has no pt-BR counterpart, so exactly one parity warning is expected.
        assert_eq!(diags.len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn crawl_on_empty_dir_returns_no_posts_and_no_diagnostics() {
        let dir = fixture_root("mev-blog-empty");
        let (posts, diags) = crawl(&dir);
        assert!(posts.is_empty());
        assert!(diags.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    // -----------------------------------------------------------------
    // BlogValidator (ContentValidator impl) end to end
    // -----------------------------------------------------------------

    #[test]
    fn blog_validator_run_reports_missing_field_and_parity() {
        let dir = fixture_root("mev-blog-validator-run");
        write_post(
            &dir,
            "broken.mdx",
            "---\ndate: 2026-01-01\nexcerpt: Summary.\n---\nBody.\n",
        );

        let report = BlogValidator::default().run(&dir);
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.locator == "E_BLOG_MISSING_FIELD"),
            "{:?}",
            report.diagnostics
        );
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.locator == "W_BLOG_PTBR_MISSING"),
            "{:?}",
            report.diagnostics
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    // -----------------------------------------------------------------
    // Funnel checks wired through BlogValidator::run (MV.12.B Task 2)
    // -----------------------------------------------------------------

    #[test]
    fn malformed_cta_target_surfaces_through_blog_validator_run() {
        let dir = fixture_root("mev-blog-funnel-malformed-cta");
        let mut body = "---\ntitle: My Post\ndate: 2026-01-01\nexcerpt: A short summary.\ncta: module\nctaTarget: not-a-valid-shape\n---\nBody text.\n".to_string();
        // Keep the rest of the checks clean so only E_FUNNEL_CTA_UNRESOLVED is under test.
        body.push('\n');
        write_post(&dir, "post.mdx", &body);

        let report = BlogValidator::default().run(&dir);
        let hits = diags_with_locator(&report.diagnostics, "E_FUNNEL_CTA_UNRESOLVED");
        assert_eq!(hits.len(), 1, "{:?}", report.diagnostics);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn clean_post_surfaces_no_funnel_codes() {
        let dir = fixture_root("mev-blog-funnel-clean");
        write_post(&dir, "post.mdx", clean_frontmatter());

        let report = BlogValidator::default().run(&dir);
        for code in [
            "E_FUNNEL_CTA_UNRESOLVED",
            "E_FUNNEL_MISSING_UTM",
            "E_FUNNEL_BARE_CAL_LINK",
            "E_FUNNEL_RAW_ANALYTICS_ATTR",
        ] {
            assert!(
                diags_with_locator(&report.diagnostics, code).is_empty(),
                "expected no {code}, got {:?}",
                report.diagnostics
            );
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn learn_root_resolves_to_none_without_sibling_learn_tree() {
        // A blog root shaped like `<content>/blog/published` with no sibling `<content>/learn`
        // directory must derive `learn_root: None` rather than pointing at a missing path, and
        // validation must still succeed (not panic, not error on the missing tree).
        let dir = crate::testsupport::unique_temp_dir("mev-blog-funnel-no-learn-tree");
        let blog_root = dir.join("content").join("blog").join("published");
        std::fs::create_dir_all(&blog_root).unwrap();
        write_post(&blog_root, "post.mdx", clean_frontmatter());

        let validator = BlogValidator::from_blog_root(&blog_root);
        assert!(
            validator.learn_root.is_none(),
            "expected no learn_root without a sibling learn tree, got {:?}",
            validator.learn_root
        );

        let report = validator.run(&blog_root);
        assert!(
            report
                .diagnostics
                .iter()
                .all(|d| d.locator != "E_FUNNEL_CTA_UNRESOLVED"),
            "{:?}",
            report.diagnostics
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
