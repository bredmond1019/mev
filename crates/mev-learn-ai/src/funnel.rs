//! Funnel conformance checks (Phase 12, Block B, Task 1).
//!
//! Four pure `(rel, source|frontmatter, ...) -> Vec<Diagnostic>` checks gating that published
//! content actually participates in the funnel:
//!
//! - [`check_cta`] — mirrors `resolveBlogCta` from `learn-ai/lib/content/cta/resolve.ts`:
//!   a `cta` frontmatter value outside the accepted vocabulary, or a `cta: module` whose
//!   `ctaTarget` is malformed or unresolvable on disk, is `E_FUNNEL_CTA_UNRESOLVED`. A post
//!   with no `cta` key at all is the legitimate default and is clean.
//! - [`check_utm`] — an `http(s)` link to `bastiel.com.br` / `bastielai.com` missing any of
//!   `utm_source` / `utm_medium` / `utm_campaign` is `E_FUNNEL_MISSING_UTM`. `mailto:`
//!   references are never URLs by this module's definition and are never flagged.
//! - [`check_cal_link`] — any `cal.com` URL in content is `E_FUNNEL_BARE_CAL_LINK`; the
//!   booking CTA renders through a component, never a hand-written link.
//! - [`check_analytics_attr`] — a raw `data-umami-*` attribute written directly into content
//!   is `E_FUNNEL_RAW_ANALYTICS_ATTR`.
//!
//! No check ever makes a network call. `check_cta`'s module-existence half is filesystem-only
//! and degrades gracefully (skips, does not error) when `learn_root` is `None`.
//!
//! The accepted `cta` vocabulary is **not hardcoded** — it lives in `data/cta-vocabulary.toml`
//! (embedded at compile time via `include_str!`, same pattern as other data-driven checks in
//! this crate) so the next variant is a one-line edit, not a release. See that file's header
//! comment for why: the vocabulary already changed once mid-spec (2026-08-06, `LA.21.C`).

use std::collections::HashSet;
use std::path::Path;
use std::sync::OnceLock;

use crate::Diagnostic;

// ---------------------------------------------------------------------------------------------
// CTA vocabulary — data-driven, embedded at compile time, overridable for tests.
// ---------------------------------------------------------------------------------------------

/// The accepted blog frontmatter `cta` values, loaded from a TOML data file rather than
/// hardcoded — see this module's doc comment.
#[derive(Debug, Clone)]
pub struct CtaVocabulary {
    values: HashSet<String>,
}

/// Typed error for a malformed vocabulary file — never a panic on bad *input*. (The shipped
/// default file is trusted at compile time; see [`default_vocabulary`].)
#[derive(Debug, thiserror::Error)]
pub enum CtaVocabularyError {
    #[error("malformed cta vocabulary TOML: {0}")]
    Toml(#[from] toml::de::Error),
}

#[derive(Debug, serde::Deserialize)]
struct RawCtaVocabulary {
    values: Vec<String>,
}

impl CtaVocabulary {
    /// Parse a vocabulary from raw TOML text — the override path tests use to prove the
    /// vocabulary is genuinely data-driven rather than hardcoded.
    pub fn parse(toml_source: &str) -> Result<Self, CtaVocabularyError> {
        let raw: RawCtaVocabulary = toml::from_str(toml_source)?;
        Ok(Self {
            values: raw.values.into_iter().collect(),
        })
    }

    /// Load the shipped default vocabulary, embedded into the binary at compile time from
    /// `data/cta-vocabulary.toml`.
    pub fn load_default() -> Result<Self, CtaVocabularyError> {
        Self::parse(DEFAULT_CTA_VOCABULARY_TOML)
    }

    /// Whether `value` is an accepted `cta` override.
    pub fn contains(&self, value: &str) -> bool {
        self.values.contains(value)
    }
}

const DEFAULT_CTA_VOCABULARY_TOML: &str = include_str!("../data/cta-vocabulary.toml");

/// The shipped vocabulary, parsed once and cached. `expect` is safe here: this is our own
/// compile-time-embedded file, not runtime input — a malformed shipped file is a build defect
/// that must fail loudly, not degrade silently at every call site. Runtime-supplied vocabularies
/// (e.g. a future CLI override) should go through [`CtaVocabulary::parse`] directly, which
/// returns a typed error instead.
fn default_vocabulary() -> &'static CtaVocabulary {
    static VOCAB: OnceLock<CtaVocabulary> = OnceLock::new();
    VOCAB.get_or_init(|| {
        CtaVocabulary::load_default()
            .expect("data/cta-vocabulary.toml (embedded at compile time) must be valid TOML")
    })
}

// ---------------------------------------------------------------------------------------------
// Frontmatter shape check_cta needs — kept minimal and independent of BlogFrontmatter so this
// module has no upward dependency on blog.rs; blog.rs (Task 2) constructs one from its own
// parsed frontmatter.
// ---------------------------------------------------------------------------------------------

/// The subset of a blog post's frontmatter [`check_cta`] needs to resolve the CTA.
#[derive(Debug, Default, Clone)]
pub struct Frontmatter {
    /// Raw `cta` frontmatter value. `None` (or blank) is the legitimate no-override default.
    pub cta: Option<String>,
    /// Raw `ctaTarget` frontmatter value, expected shape `"<path-slug>/<module-id>"`.
    pub cta_target: Option<String>,
}

// ---------------------------------------------------------------------------------------------
// check_cta
// ---------------------------------------------------------------------------------------------

/// Resolve `fm`'s `cta` override against the shipped vocabulary and, for `cta: module`,
/// against the on-disk module tree rooted at `learn_root`.
///
/// Mirrors `resolveBlogCta`, with one deliberate divergence: `resolveBlogCta` is total and
/// silently degrades a malformed override to `{ kind: 'newsletter' }` so a route never 500s;
/// this checker exists precisely to catch that silent degradation, so every case that would
/// degrade emits `E_FUNNEL_CTA_UNRESOLVED` instead of passing quietly.
pub fn check_cta(rel: &Path, fm: &Frontmatter, learn_root: Option<&Path>) -> Vec<Diagnostic> {
    check_cta_with_vocab(rel, fm, default_vocabulary(), learn_root)
}

/// Same as [`check_cta`] but against an explicit vocabulary — the seam tests use to prove the
/// vocabulary is genuinely data-driven (a value absent from the shipped list but present in an
/// override list resolves clean, and vice versa).
pub fn check_cta_with_vocab(
    rel: &Path,
    fm: &Frontmatter,
    vocab: &CtaVocabulary,
    learn_root: Option<&Path>,
) -> Vec<Diagnostic> {
    let cta = match fm.cta.as_deref().map(str::trim) {
        Some(v) if !v.is_empty() => v,
        _ => return Vec::new(), // no cta key (or blank) -> legitimate newsletter default
    };

    if !vocab.contains(cta) {
        return vec![Diagnostic::error(
            rel.to_path_buf(),
            "E_FUNNEL_CTA_UNRESOLVED",
            format!(
                "unrecognized cta value `{cta}` (accepted vocabulary: data/cta-vocabulary.toml)"
            ),
        )];
    }

    if cta == "module" {
        let target = fm.cta_target.as_deref().map(str::trim).unwrap_or("");
        let parts: Vec<&str> = target.split('/').collect();
        let malformed =
            target.is_empty() || parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty();
        if malformed {
            return vec![Diagnostic::error(
                rel.to_path_buf(),
                "E_FUNNEL_CTA_UNRESOLVED",
                format!(
                    "cta: module requires ctaTarget of shape \"<path-slug>/<module-id>\", got `{target}`"
                ),
            )];
        }

        let (path_slug, module_id) = (parts[0], parts[1]);
        if let Some(learn_root) = learn_root
            && !module_exists(learn_root, path_slug, module_id)
        {
            return vec![Diagnostic::error(
                rel.to_path_buf(),
                "E_FUNNEL_CTA_UNRESOLVED",
                format!("ctaTarget `{path_slug}/{module_id}` does not resolve to a module on disk"),
            )];
        }
    }

    Vec::new()
}

/// `true` if `<learn_root>/paths/<path_slug>/modules/` contains a file whose stem, after
/// stripping a leading `NN-` numeric-order prefix, equals `module_id` — the shape module ids
/// take in `ctaTarget` (e.g. file `01-rethinking-agents-as-software.mdx` matches module id
/// `rethinking-agents-as-software`). A missing/unreadable directory is simply "no match", not
/// an error.
fn module_exists(learn_root: &Path, path_slug: &str, module_id: &str) -> bool {
    let dir = learn_root.join("paths").join(path_slug).join("modules");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return false;
    };
    entries.filter_map(Result::ok).any(|entry| {
        entry
            .path()
            .file_stem()
            .and_then(|s| s.to_str())
            .map(strip_numeric_prefix)
            == Some(module_id)
    })
}

/// Strip a leading `NN-` (one or more ASCII digits, then `-`) order prefix from a file stem.
/// A stem with no such prefix is returned unchanged.
fn strip_numeric_prefix(stem: &str) -> &str {
    let digit_len = stem.bytes().take_while(u8::is_ascii_digit).count();
    if digit_len > 0 && stem.as_bytes().get(digit_len) == Some(&b'-') {
        &stem[digit_len + 1..]
    } else {
        stem
    }
}

// ---------------------------------------------------------------------------------------------
// check_utm / check_cal_link / check_analytics_attr — pure text scans, no network calls.
// ---------------------------------------------------------------------------------------------

/// Emit `E_FUNNEL_MISSING_UTM` for every `http(s)` link to `bastiel.com.br` / `bastielai.com`
/// (including subdomains) missing any of `utm_source`, `utm_medium`, `utm_campaign`.
///
/// `mailto:` references are never matched — [`extract_urls`] only recognizes `http://` /
/// `https://` prefixes, so a bare-domain match (which would fire on all 12 live
/// `mailto:brandon@bastiel.com.br` references) is structurally impossible here.
pub fn check_utm(rel: &Path, source: &str) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for (idx, line) in source.lines().enumerate() {
        let line_no = idx + 1;
        for url in extract_urls(line) {
            let Some(host) = url_host(&url) else {
                continue;
            };
            if !is_bastiel_host(host) {
                continue;
            }
            let missing = missing_utm_params(&url);
            if !missing.is_empty() {
                diags.push(Diagnostic::error(
                    rel.to_path_buf(),
                    "E_FUNNEL_MISSING_UTM",
                    format!(
                        "bastiel link `{url}` at line {line_no} is missing UTM param(s): {}",
                        missing.join(", ")
                    ),
                ));
            }
        }
    }
    diags
}

/// Emit `E_FUNNEL_BARE_CAL_LINK` for every `cal.com` URL found in `source`.
pub fn check_cal_link(rel: &Path, source: &str) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for (idx, line) in source.lines().enumerate() {
        let line_no = idx + 1;
        for url in extract_urls(line) {
            if let Some(host) = url_host(&url)
                && (host == "cal.com" || host.ends_with(".cal.com"))
            {
                diags.push(Diagnostic::error(
                    rel.to_path_buf(),
                    "E_FUNNEL_BARE_CAL_LINK",
                    format!(
                        "bare cal.com link `{url}` at line {line_no} — the booking CTA renders \
                         through the component, never a hand-written link"
                    ),
                ));
            }
        }
    }
    diags
}

/// Emit `E_FUNNEL_RAW_ANALYTICS_ATTR` for every line carrying a raw `data-umami-*` attribute.
pub fn check_analytics_attr(rel: &Path, source: &str) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for (idx, line) in source.lines().enumerate() {
        let line_no = idx + 1;
        if line.contains("data-umami-") {
            diags.push(Diagnostic::error(
                rel.to_path_buf(),
                "E_FUNNEL_RAW_ANALYTICS_ATTR",
                format!(
                    "raw data-umami-* attribute at line {line_no} — use learn-ai's analytics \
                     module instead of a hand-written attribute"
                ),
            ));
        }
    }
    diags
}

/// Extract every substring of `line` that looks like an `http://`/`https://` URL: starts at
/// the scheme and runs until whitespace or a character that commonly closes a URL in markdown
/// prose (`) ] " ' \` >` or a trailing comma). Deliberately does not match `mailto:` or any
/// other scheme — the funnel checks only ever care about real outbound web links.
fn extract_urls(line: &str) -> Vec<String> {
    let mut urls = Vec::new();
    let mut search_from = 0usize;

    loop {
        let rest = &line[search_from..];
        let http_pos = rest.find("http://");
        let https_pos = rest.find("https://");
        let start = match (http_pos, https_pos) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        };
        let Some(start) = start else { break };

        let abs_start = search_from + start;
        let candidate = &line[abs_start..];
        let end = candidate
            .find(|c: char| {
                c.is_whitespace() || matches!(c, ')' | ']' | '"' | '\'' | '>' | '`' | ',')
            })
            .unwrap_or(candidate.len());
        let url = &candidate[..end];
        urls.push(url.to_string());

        search_from = abs_start + end.max(1);
        if search_from >= line.len() {
            break;
        }
    }

    urls
}

/// Extract the host from an `http://`/`https://` URL — everything after the scheme up to the
/// first `/`, `:`, `?`, or `#`. `None` if `url` has neither scheme (should not happen for
/// anything [`extract_urls`] produced).
fn url_host(url: &str) -> Option<&str> {
    let after_scheme = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let end = after_scheme
        .find(['/', ':', '?', '#'])
        .unwrap_or(after_scheme.len());
    Some(&after_scheme[..end])
}

/// `true` for `bastiel.com.br` / `bastielai.com` or any subdomain of either.
fn is_bastiel_host(host: &str) -> bool {
    host == "bastiel.com.br"
        || host.ends_with(".bastiel.com.br")
        || host == "bastielai.com"
        || host.ends_with(".bastielai.com")
}

/// Return the names, among `utm_source`/`utm_medium`/`utm_campaign`, absent from `url`'s query
/// string. Presence-and-shape only — never validates the *value* against a campaign registry
/// (explicitly out of scope for this block).
fn missing_utm_params(url: &str) -> Vec<&'static str> {
    let query = url.split_once('?').map(|(_, q)| q).unwrap_or("");
    ["utm_source", "utm_medium", "utm_campaign"]
        .into_iter()
        .filter(|name| {
            !query
                .split('&')
                .any(|kv| kv.starts_with(&format!("{name}=")))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn rel(name: &str) -> PathBuf {
        PathBuf::from(name)
    }

    fn locators(diags: &[Diagnostic]) -> Vec<&str> {
        diags.iter().map(|d| d.locator.as_str()).collect()
    }

    // -----------------------------------------------------------------------------------
    // check_cta — vocabulary + no-key default
    // -----------------------------------------------------------------------------------

    #[test]
    fn no_cta_key_is_clean() {
        let fm = Frontmatter::default();
        let diags = check_cta(&rel("post.mdx"), &fm, None);
        assert!(diags.is_empty(), "{diags:?}");
    }

    #[test]
    fn blank_cta_is_treated_as_no_key() {
        let fm = Frontmatter {
            cta: Some("   ".to_string()),
            cta_target: None,
        };
        let diags = check_cta(&rel("post.mdx"), &fm, None);
        assert!(diags.is_empty(), "{diags:?}");
    }

    #[test]
    fn newsletter_cta_is_clean() {
        let fm = Frontmatter {
            cta: Some("newsletter".to_string()),
            cta_target: None,
        };
        let diags = check_cta(&rel("post.mdx"), &fm, None);
        assert!(diags.is_empty(), "{diags:?}");
    }

    #[test]
    fn bastiel_cta_resolves_clean_against_shipped_vocabulary() {
        // Pins the 2026-08-06 vocabulary change: `bastiel` must resolve clean via the
        // *shipped* data file, not a hardcoded list.
        let fm = Frontmatter {
            cta: Some("bastiel".to_string()),
            cta_target: None,
        };
        let diags = check_cta(&rel("post.mdx"), &fm, None);
        assert!(diags.is_empty(), "{diags:?}");
    }

    #[test]
    fn unrecognized_cta_value_is_unresolved() {
        let fm = Frontmatter {
            cta: Some("webinar".to_string()),
            cta_target: None,
        };
        let diags = check_cta(&rel("post.mdx"), &fm, None);
        assert_eq!(
            locators(&diags),
            vec!["E_FUNNEL_CTA_UNRESOLVED"],
            "{diags:?}"
        );
    }

    // -----------------------------------------------------------------------------------
    // check_cta — cta: module, malformed ctaTarget shapes
    // -----------------------------------------------------------------------------------

    #[test]
    fn module_cta_with_missing_target_is_unresolved() {
        let fm = Frontmatter {
            cta: Some("module".to_string()),
            cta_target: None,
        };
        let diags = check_cta(&rel("post.mdx"), &fm, None);
        assert_eq!(
            locators(&diags),
            vec!["E_FUNNEL_CTA_UNRESOLVED"],
            "{diags:?}"
        );
    }

    #[test]
    fn module_cta_with_no_slash_is_unresolved() {
        let fm = Frontmatter {
            cta: Some("module".to_string()),
            cta_target: Some("just-a-path-slug".to_string()),
        };
        let diags = check_cta(&rel("post.mdx"), &fm, None);
        assert_eq!(
            locators(&diags),
            vec!["E_FUNNEL_CTA_UNRESOLVED"],
            "{diags:?}"
        );
    }

    #[test]
    fn module_cta_with_empty_segment_is_unresolved() {
        let fm = Frontmatter {
            cta: Some("module".to_string()),
            cta_target: Some("path-slug/".to_string()),
        };
        let diags = check_cta(&rel("post.mdx"), &fm, None);
        assert_eq!(
            locators(&diags),
            vec!["E_FUNNEL_CTA_UNRESOLVED"],
            "{diags:?}"
        );
    }

    #[test]
    fn module_cta_with_too_many_segments_is_unresolved() {
        let fm = Frontmatter {
            cta: Some("module".to_string()),
            cta_target: Some("a/b/c".to_string()),
        };
        let diags = check_cta(&rel("post.mdx"), &fm, None);
        assert_eq!(
            locators(&diags),
            vec!["E_FUNNEL_CTA_UNRESOLVED"],
            "{diags:?}"
        );
    }

    // -----------------------------------------------------------------------------------
    // check_cta — module existence, learn_root Some vs None
    // -----------------------------------------------------------------------------------

    fn write_module(learn_root: &Path, path_slug: &str, filename: &str) {
        let dir = learn_root.join("paths").join(path_slug).join("modules");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(filename), "content").unwrap();
    }

    #[test]
    fn module_cta_with_resolvable_target_is_clean() {
        let dir = crate::testsupport::unique_temp_dir("mev-funnel-module-resolve");
        write_module(
            &dir,
            "12-factor-agent-development",
            "01-rethinking-agents-as-software.mdx",
        );

        let fm = Frontmatter {
            cta: Some("module".to_string()),
            cta_target: Some(
                "12-factor-agent-development/rethinking-agents-as-software".to_string(),
            ),
        };
        let diags = check_cta(&rel("post.mdx"), &fm, Some(&dir));
        assert!(diags.is_empty(), "{diags:?}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn module_cta_with_unresolvable_target_is_unresolved() {
        let dir = crate::testsupport::unique_temp_dir("mev-funnel-module-unresolvable");
        write_module(
            &dir,
            "12-factor-agent-development",
            "01-rethinking-agents-as-software.mdx",
        );

        let fm = Frontmatter {
            cta: Some("module".to_string()),
            cta_target: Some("12-factor-agent-development/does-not-exist".to_string()),
        };
        let diags = check_cta(&rel("post.mdx"), &fm, Some(&dir));
        assert_eq!(
            locators(&diags),
            vec!["E_FUNNEL_CTA_UNRESOLVED"],
            "{diags:?}"
        );
        assert!(diags[0].message.contains("does-not-exist"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn module_cta_with_learn_root_none_skips_existence_check_only() {
        // Well-formed target, no learn tree available — must not flag; only the shape check
        // (already proven above) still runs.
        let fm = Frontmatter {
            cta: Some("module".to_string()),
            cta_target: Some("some-path/some-module".to_string()),
        };
        let diags = check_cta(&rel("post.mdx"), &fm, None);
        assert!(diags.is_empty(), "{diags:?}");
    }

    // -----------------------------------------------------------------------------------
    // check_cta — data-driven vocabulary seam
    // -----------------------------------------------------------------------------------

    #[test]
    fn value_absent_from_shipped_list_but_present_in_override_resolves_clean() {
        let custom = CtaVocabulary::parse(r#"values = ["newsletter", "webinar"]"#).unwrap();
        let fm = Frontmatter {
            cta: Some("webinar".to_string()),
            cta_target: None,
        };
        let diags = check_cta_with_vocab(&rel("post.mdx"), &fm, &custom, None);
        assert!(
            diags.is_empty(),
            "override vocabulary should make `webinar` resolve clean: {diags:?}"
        );
    }

    #[test]
    fn value_present_in_shipped_list_but_absent_from_override_is_flagged() {
        let custom = CtaVocabulary::parse(r#"values = ["newsletter", "webinar"]"#).unwrap();
        let fm = Frontmatter {
            cta: Some("bastiel".to_string()),
            cta_target: None,
        };
        let diags = check_cta_with_vocab(&rel("post.mdx"), &fm, &custom, None);
        assert_eq!(
            locators(&diags),
            vec!["E_FUNNEL_CTA_UNRESOLVED"],
            "override vocabulary should flag `bastiel` when it is not in the override list: {diags:?}"
        );
    }

    #[test]
    fn malformed_vocabulary_toml_is_a_typed_error_not_a_panic() {
        let result = CtaVocabulary::parse("this is not valid toml [[[");
        assert!(matches!(result, Err(CtaVocabularyError::Toml(_))));
    }

    // -----------------------------------------------------------------------------------
    // check_utm
    // -----------------------------------------------------------------------------------

    #[test]
    fn bastiel_url_with_all_three_utm_params_is_clean() {
        let source = "Visit https://bastiel.com.br/en?utm_source=blog&utm_medium=post&utm_campaign=launch today.";
        let diags = check_utm(&rel("post.mdx"), source);
        assert!(diags.is_empty(), "{diags:?}");
    }

    #[test]
    fn bastiel_url_with_two_of_three_utm_params_is_flagged() {
        let source = "Visit https://bastiel.com.br/en?utm_source=blog&utm_medium=post today.";
        let diags = check_utm(&rel("post.mdx"), source);
        assert_eq!(locators(&diags), vec!["E_FUNNEL_MISSING_UTM"], "{diags:?}");
        assert!(diags[0].message.contains("utm_campaign"));
    }

    #[test]
    fn bastiel_url_with_no_utm_params_is_flagged() {
        let source = "Visit https://bastiel.com.br/en today.";
        let diags = check_utm(&rel("post.mdx"), source);
        assert_eq!(locators(&diags), vec!["E_FUNNEL_MISSING_UTM"], "{diags:?}");
    }

    #[test]
    fn bastielai_subdomain_is_also_checked() {
        let source = "Visit https://app.bastielai.com/start today.";
        let diags = check_utm(&rel("post.mdx"), source);
        assert_eq!(locators(&diags), vec!["E_FUNNEL_MISSING_UTM"], "{diags:?}");
    }

    #[test]
    fn mailto_bastiel_reference_produces_no_diagnostic() {
        // Pins the trap the spec calls out: all live `bastiel` occurrences in content today
        // are `mailto:` addresses, and a bare-domain match would fire 14 false positives.
        let source = "Questions? Email mailto:brandon@bastiel.com.br for details.";
        let diags = check_utm(&rel("post.mdx"), source);
        assert!(diags.is_empty(), "{diags:?}");
    }

    #[test]
    fn non_bastiel_url_is_ignored_by_utm_check() {
        let source = "See https://example.com/page?utm_source=x for more.";
        let diags = check_utm(&rel("post.mdx"), source);
        assert!(diags.is_empty(), "{diags:?}");
    }

    // -----------------------------------------------------------------------------------
    // check_cal_link
    // -----------------------------------------------------------------------------------

    #[test]
    fn cal_com_link_is_flagged() {
        let source = "Book here: https://cal.com/brandon-redmond/30min-diagnostico-ia";
        let diags = check_cal_link(&rel("post.mdx"), source);
        assert_eq!(
            locators(&diags),
            vec!["E_FUNNEL_BARE_CAL_LINK"],
            "{diags:?}"
        );
    }

    #[test]
    fn non_cal_link_is_clean() {
        let source =
            "Book here: https://bastiel.com.br/en?utm_source=x&utm_medium=y&utm_campaign=z";
        let diags = check_cal_link(&rel("post.mdx"), source);
        assert!(diags.is_empty(), "{diags:?}");
    }

    // -----------------------------------------------------------------------------------
    // check_analytics_attr
    // -----------------------------------------------------------------------------------

    #[test]
    fn raw_data_umami_attribute_is_flagged() {
        let source = "<button data-umami-event=\"cta-click\">Book</button>";
        let diags = check_analytics_attr(&rel("post.mdx"), source);
        assert_eq!(
            locators(&diags),
            vec!["E_FUNNEL_RAW_ANALYTICS_ATTR"],
            "{diags:?}"
        );
    }

    #[test]
    fn content_without_umami_attribute_is_clean() {
        let source = "<button onClick={trackClick}>Book</button>";
        let diags = check_analytics_attr(&rel("post.mdx"), source);
        assert!(diags.is_empty(), "{diags:?}");
    }
}
