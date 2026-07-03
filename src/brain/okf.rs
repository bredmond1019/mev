//! OKF frontmatter validation for the company-brain repo (Phase 2, Block H).
//!
//! [`OkfFrontmatter`] is re-exported from bastion's `okf-core` crate (BA.15.12 /
//! D15/D16 format convergence) — this module no longer defines its own struct.
//! [`validate_md_file`] checks every required field, controlled-vocab membership,
//! kebab-case `doc_id`, and keyword count on a single brain `.md` file, against
//! that shared model.
//!
//! Design mirrors `learn_ai::meta`: read/parse failures short-circuit to a single
//! diagnostic, and every field violation gets its own precise-locator diagnostic.

use crate::Diagnostic;
use crate::brain::config::BrainConfig;
use crate::brain::crawl::MdFile;
use crate::shared::{extract_frontmatter, non_empty};

// ---------------------------------------------------------------------------
// OKF frontmatter struct — delegated to okf-core (BA.15.12 convergence)
// ---------------------------------------------------------------------------

/// The OKF YAML frontmatter schema (D27) for company-brain `.md` files.
///
/// Single source of truth: [`okf_core::OkfFrontmatter`] (BA.15.12/D15/D16). The three
/// required scalars (`type_`/`title`/`description`) and the optional `doc_id`/`project`/
/// `status`/`synced_from` scalars are `Option<String>`; `layer`/`keywords`/`related` are
/// `Vec<String>` (empty means absent — deserialization defaults absent keys to `vec![]`,
/// not `None`), unlike the `Option<Vec<String>>` shape this module used before the
/// convergence. Per-field checks below account for that shape.
pub use okf_core::OkfFrontmatter;

// ---------------------------------------------------------------------------
// Controlled-vocabulary sets (config-driven — no hardcoded literal arrays)
// ---------------------------------------------------------------------------

/// `true` if `s` is in the `vocab.layer` list from `brain.toml`.
pub(crate) fn is_valid_layer(s: &str, config: &BrainConfig) -> bool {
    config.vocab.layer.iter().any(|v| v == s)
}

/// `true` if `s` is the slug of any `[[repos]]` entry in `brain.toml`.
pub(crate) fn is_valid_project(s: &str, config: &BrainConfig) -> bool {
    config.projects().contains(&s)
}

/// `true` if `s` is in the `vocab.status` list from `brain.toml`.
pub(crate) fn is_valid_status(s: &str, config: &BrainConfig) -> bool {
    config.vocab.status.iter().any(|v| v == s)
}

// ---------------------------------------------------------------------------
// doc_id helpers
// ---------------------------------------------------------------------------

/// `true` if `s` matches the Brain decision-file convention: `D` followed by one or more
/// digits, optionally followed by lowercase kebab segments (e.g. `D15-okf-lowercase-doc-names`).
pub(crate) fn is_decision_id(s: &str) -> bool {
    let rest = match s.strip_prefix('D') {
        Some(r) => r,
        None => return false,
    };
    // Must have at least one digit after 'D'.
    let rest = match rest.find(|c: char| !c.is_ascii_digit()) {
        Some(0) => return false, // nothing consumed — no digit follows 'D'
        Some(i) => &rest[i..],
        None if !rest.is_empty() => return true, // "D7", "D29" — digits only
        None => return false,                    // rest is empty → bare "D"
    };
    // Remaining portion (if any) must be `-<kebab-segment>+`.
    rest.strip_prefix('-')
        .map(|tail| {
            // Every hyphen-separated segment is non-empty all-lowercase-or-digit.
            tail.split('-').all(|seg| {
                !seg.is_empty() && seg.chars().all(|c| matches!(c, 'a'..='z' | '0'..='9'))
            })
        })
        .unwrap_or(false)
}

/// `true` if `s` is a valid OKF `doc_id`: either standard kebab-case or a decision file id.
pub(crate) fn is_valid_doc_id(s: &str) -> bool {
    crate::shared::is_kebab_case(s) || is_decision_id(s)
}

// ---------------------------------------------------------------------------
// Per-file helpers
// ---------------------------------------------------------------------------

/// Return `true` if `mf` is a unit-root instruction file (`README.md` or `CLAUDE.md`
/// located exactly at its owning scope unit's root, not deep in `docs/` or `planning/`).
///
/// Root instruction files at the unit root are valid corpus leaves but are **not** required
/// to carry OKF frontmatter.  A `docs/README.md` or `planning/CLAUDE.md` is a normal corpus
/// member and must be validated; the name-only check is insufficient.
pub(crate) fn is_root_instruction_file(mf: &MdFile, config: &BrainConfig) -> bool {
    // Fast-path: wrong filename.
    if !matches!(
        mf.path.file_name().and_then(|n| n.to_str()),
        Some("README.md") | Some("CLAUDE.md")
    ) {
        return false;
    }
    // Verify the file sits exactly at its owning unit's root (not under docs/ or planning/).
    let (_, unit_repo_path) = crate::brain::scope::owning_unit(&mf.rel, config);
    let trimmed = unit_repo_path.trim();
    let unit_rel: &std::path::Path = if trimmed == "." || trimmed.is_empty() {
        &mf.rel
    } else {
        match mf.rel.strip_prefix(trimmed) {
            Ok(r) => r,
            Err(_) => return false,
        }
    };
    unit_rel == std::path::Path::new("README.md") || unit_rel == std::path::Path::new("CLAUDE.md")
}

/// Build a "missing required field" `error` diagnostic.
fn missing(mf: &MdFile, locator: &str) -> Diagnostic {
    Diagnostic::error(mf.rel.clone(), locator, "missing required field")
}

/// Push a `missing` diagnostic when `value` is absent or blank.
fn require_str(mf: &MdFile, locator: &str, value: &Option<String>, diags: &mut Vec<Diagnostic>) {
    if non_empty(value).is_none() {
        diags.push(missing(mf, locator));
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Validate the OKF frontmatter of a single brain `.md` file.
///
/// Returns every diagnostic the file produces; an empty vector means the file is clean.
///
/// **OKF exemption for root instruction files:** `README.md` and `CLAUDE.md` are valid
/// corpus leaves but are **not** required to carry OKF frontmatter.  When such a file has
/// no frontmatter, this function returns an empty diagnostic list (no error).  If the file
/// *does* carry frontmatter, it is validated normally like any other corpus file.
///
/// Error short-circuits:
/// - File read failure → one `error` diagnostic.
/// - No frontmatter block (non-exempt file) → one `error` diagnostic at locator `"frontmatter"`.
/// - Malformed YAML → one `error` diagnostic at locator `"frontmatter"`.
///
/// Field checks (after successful parse):
/// - Missing `type`, `title`, or `description` → one `error` each at that locator.
/// - `type` value is **never** vocab-checked (open vocab; presence only).
/// - Bad `layer` member → one `error` per bad member at locator `"layer"`.
/// - Bad `project` value → one `error` at locator `"project"`.
/// - Bad `status` value → one `error` at locator `"status"`.
/// - Non-kebab-case `doc_id` (when present) → one `error` at locator `"doc_id"`.
/// - `keywords` count outside 3–7 (when present) → one `warning` at locator `"keywords"`.
pub fn validate_md_file(mf: &MdFile, config: &BrainConfig) -> Vec<Diagnostic> {
    // --- Read ---
    let contents = match std::fs::read_to_string(&mf.path) {
        Ok(s) => s,
        Err(e) => {
            return vec![Diagnostic::error(
                mf.rel.clone(),
                "",
                format!("could not read file: {e}"),
            )];
        }
    };

    // --- Extract frontmatter ---
    let yaml = match extract_frontmatter(&contents) {
        Some(y) => y,
        None => {
            // Root instruction files (README.md / CLAUDE.md) without frontmatter are
            // valid corpus leaves — they must not raise the OKF "missing frontmatter" error.
            if is_root_instruction_file(mf, config) {
                return vec![];
            }
            return vec![Diagnostic::error(
                mf.rel.clone(),
                "frontmatter",
                "missing or unterminated frontmatter block (expected leading --- fence)",
            )];
        }
    };

    // --- Parse YAML ---
    let fm: OkfFrontmatter = match serde_yaml::from_str(yaml) {
        Ok(f) => f,
        Err(e) => {
            return vec![Diagnostic::error(
                mf.rel.clone(),
                "frontmatter",
                format!("malformed YAML in frontmatter: {e}"),
            )];
        }
    };

    let mut diags = Vec::new();

    // --- Required fields: type, title, description ---
    require_str(mf, "type", &fm.type_, &mut diags);
    require_str(mf, "title", &fm.title, &mut diags);
    require_str(mf, "description", &fm.description, &mut diags);

    // --- Controlled vocab: layer (list — each bad member is its own error) ---
    // `fm.layer` is `Vec<String>` (empty means absent) since the okf-core convergence;
    // iterating an empty vec is a no-op, preserving the prior `Option`-absent behavior.
    for layer in &fm.layer {
        if !is_valid_layer(layer.as_str(), config) {
            diags.push(Diagnostic::error(
                mf.rel.clone(),
                "layer",
                format!(
                    "layer value '{layer}' is not in the configured vocabulary \
                     ({})",
                    config.vocab.layer.join("|")
                ),
            ));
        }
    }

    // --- Controlled vocab: project (scalar; absent is OK) ---
    if let Some(project) = non_empty(&fm.project)
        && !is_valid_project(project, config)
    {
        diags.push(Diagnostic::error(
            mf.rel.clone(),
            "project",
            format!(
                "project value '{project}' is not in the configured vocabulary \
                 ({})",
                config.projects().join("|")
            ),
        ));
    }

    // --- Controlled vocab: status (scalar; absent is OK) ---
    if let Some(status) = non_empty(&fm.status)
        && !is_valid_status(status, config)
    {
        diags.push(Diagnostic::error(
            mf.rel.clone(),
            "status",
            format!(
                "status value '{status}' is not in the configured vocabulary \
                 ({})",
                config.vocab.status.join("|")
            ),
        ));
    }

    // --- doc_id: kebab-case or decision-id (D\d+…) when present ---
    if let Some(doc_id) = non_empty(&fm.doc_id)
        && !is_valid_doc_id(doc_id)
    {
        diags.push(Diagnostic::error(
            mf.rel.clone(),
            "doc_id",
            format!("doc_id must be kebab-case or a decision id (D<n>-…): {doc_id}"),
        ));
    }

    // --- keywords: count 3–7 warning when present ---
    // Empty `Vec` means absent (see the module doc comment) — must not be flagged, same
    // as the prior `Option::None` behavior.
    if !fm.keywords.is_empty() {
        let count = fm.keywords.len();
        if !(3..=7).contains(&count) {
            diags.push(Diagnostic::warning(
                mf.rel.clone(),
                "keywords",
                format!("keywords should contain 3–7 entries (found {count})"),
            ));
        }
    }

    diags
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Severity;
    use crate::brain::config::{BrainConfig, CrawlConfig, RepoEntry, VocabConfig};
    use crate::brain::crawl::MdFile;
    use std::path::PathBuf;

    // --- Helpers ---

    fn temp_dir(suffix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mev-okf-{suffix}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_md(dir: &PathBuf, name: &str, content: &str) -> MdFile {
        let path = dir.join(name);
        std::fs::write(&path, content).unwrap();
        MdFile {
            path: path.clone(),
            rel: PathBuf::from(name),
            stem: name.trim_end_matches(".md").to_string(),
        }
    }

    /// Build a [`BrainConfig`] covering the full standard vocabulary — used in tests
    /// that need valid layer/status/project values recognised.
    fn full_test_config() -> BrainConfig {
        let layer = [
            "brain", "engine", "factory", "console", "surface", "infra", "business", "content",
            "meta",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let status = ["active", "draft", "deprecated", "superseded", "archived"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let slugs = [
            "bastion",
            "bastion-ui",
            "orchestrator",
            "learn-ai",
            "rag-engine-rs",
            "claude-sdk-rs",
            "workflow-engine-rs",
            "mev",
            "bella",
            "price-scout",
            "amistad",
            "base-template",
            "brain",
        ];
        let repos = slugs
            .iter()
            .map(|slug| RepoEntry {
                slug: slug.to_string(),
                tier: String::new(),
                repo_path: String::new(),
                status_file: String::new(),
                cache_doc: String::new(),
                heading: String::new(),
            })
            .collect();
        BrainConfig {
            vocab: VocabConfig { layer, status },
            crawl: CrawlConfig::default(),
            repos,
        }
    }

    /// A fully-valid OKF frontmatter body (all required + optional fields).
    fn good_okf_body() -> &'static str {
        "---\ntype: Decision\ntitle: My Decision\ndescription: A one-line summary.\ndoc_id: my-decision\nlayer: [brain]\nproject: bastion\nstatus: active\nkeywords: [rust, cli, validation]\nrelated: [context]\n---\n\n# Body\n"
    }

    /// Validate a YAML string by writing to a temp file, using the full standard config.
    fn validate_yaml(yaml_body: &str, suffix: &str) -> Vec<Diagnostic> {
        validate_yaml_with_config(yaml_body, suffix, &full_test_config())
    }

    /// Validate a YAML string with an explicit config.
    fn validate_yaml_with_config(
        yaml_body: &str,
        suffix: &str,
        config: &BrainConfig,
    ) -> Vec<Diagnostic> {
        let dir = temp_dir(suffix);
        let mf = write_md(&dir, "test.md", yaml_body);
        let diags = validate_md_file(&mf, config);
        let _ = std::fs::remove_dir_all(&dir);
        diags
    }

    // --- Vocab helpers ---

    #[test]
    fn layer_helper_accepts_all_valid_values() {
        let cfg = full_test_config();
        for v in [
            "brain", "engine", "factory", "console", "surface", "infra", "business", "content",
            "meta",
        ] {
            assert!(is_valid_layer(v, &cfg), "expected '{v}' to be valid");
        }
    }

    #[test]
    fn layer_helper_rejects_unknown_value() {
        let cfg = full_test_config();
        assert!(!is_valid_layer("unknown", &cfg));
        assert!(!is_valid_layer("Brain", &cfg)); // case-sensitive
        assert!(!is_valid_layer("", &cfg));
    }

    #[test]
    fn project_helper_accepts_all_valid_values() {
        let cfg = full_test_config();
        for v in [
            "bastion",
            "bastion-ui",
            "orchestrator",
            "learn-ai",
            "rag-engine-rs",
            "claude-sdk-rs",
            "workflow-engine-rs",
            "mev",
            "bella",
            "price-scout",
            "amistad",
            "base-template",
            "brain",
        ] {
            assert!(is_valid_project(v, &cfg), "expected '{v}' to be valid");
        }
    }

    #[test]
    fn project_helper_rejects_unknown_value() {
        let cfg = full_test_config();
        assert!(!is_valid_project("unknown-project", &cfg));
        assert!(!is_valid_project("Bastion", &cfg)); // case-sensitive
    }

    #[test]
    fn status_helper_accepts_all_valid_values() {
        let cfg = full_test_config();
        for v in ["active", "draft", "deprecated", "superseded", "archived"] {
            assert!(is_valid_status(v, &cfg), "expected '{v}' to be valid");
        }
    }

    #[test]
    fn status_helper_rejects_unknown_value() {
        let cfg = full_test_config();
        assert!(!is_valid_status("pending", &cfg));
        assert!(!is_valid_status("Active", &cfg)); // case-sensitive
    }

    // --- Good document is clean ---

    #[test]
    fn good_okf_document_is_clean() {
        let diags = validate_yaml(good_okf_body(), "good");
        assert!(diags.is_empty(), "expected no diagnostics, got: {diags:?}");
    }

    #[test]
    fn doc_with_synced_from_validates_clean() {
        // A brain cache doc that carries `synced_from` must still pass OKF validation — the
        // field is tolerated (presence-only) and not format-checked by the OKF schema.
        let body = "---\ntype: ProjectContext\ntitle: Cache\ndescription: Brain cache doc.\nsynced_from: \"2026-06-27T12:00:00+00:00\"\n---\nbody\n";
        let diags = validate_yaml(body, "synced-from");
        assert!(
            diags.is_empty(),
            "doc with synced_from should validate clean, got: {diags:?}"
        );
    }

    // --- Required fields ---

    #[test]
    fn missing_type_emits_error_at_type_locator() {
        let body = "---\ntitle: T\ndescription: D\n---\nbody\n";
        let diags = validate_yaml(body, "no-type");
        assert_eq!(diags.len(), 1, "got: {diags:?}");
        assert_eq!(diags[0].severity, Severity::Error);
        assert_eq!(diags[0].locator, "type");
        assert_eq!(diags[0].message, "missing required field");
    }

    #[test]
    fn missing_title_emits_error_at_title_locator() {
        let body = "---\ntype: Decision\ndescription: D\n---\nbody\n";
        let diags = validate_yaml(body, "no-title");
        assert_eq!(diags.len(), 1, "got: {diags:?}");
        assert_eq!(diags[0].severity, Severity::Error);
        assert_eq!(diags[0].locator, "title");
    }

    #[test]
    fn missing_description_emits_error_at_description_locator() {
        let body = "---\ntype: Decision\ntitle: T\n---\nbody\n";
        let diags = validate_yaml(body, "no-desc");
        assert_eq!(diags.len(), 1, "got: {diags:?}");
        assert_eq!(diags[0].severity, Severity::Error);
        assert_eq!(diags[0].locator, "description");
    }

    #[test]
    fn type_value_is_not_vocab_checked() {
        // Any non-empty type value must be accepted.
        let body = "---\ntype: SomeArbitraryType\ntitle: T\ndescription: D\n---\nbody\n";
        let diags = validate_yaml(body, "any-type");
        assert!(
            diags.is_empty(),
            "type value should not be vocab-checked, got: {diags:?}"
        );
    }

    // --- Controlled vocab: layer ---

    #[test]
    fn bad_layer_member_emits_error_at_layer_locator() {
        let body =
            "---\ntype: T\ntitle: T\ndescription: D\nlayer: [brain, invalid-layer]\n---\nbody\n";
        let diags = validate_yaml(body, "bad-layer");
        assert_eq!(diags.len(), 1, "got: {diags:?}");
        assert_eq!(diags[0].severity, Severity::Error);
        assert_eq!(diags[0].locator, "layer");
        assert!(diags[0].message.contains("invalid-layer"));
    }

    #[test]
    fn two_bad_layer_members_emit_two_errors() {
        let body = "---\ntype: T\ntitle: T\ndescription: D\nlayer: [bad1, bad2]\n---\nbody\n";
        let diags = validate_yaml(body, "two-bad-layers");
        assert_eq!(diags.len(), 2, "got: {diags:?}");
        assert!(diags.iter().all(|d| d.locator == "layer"));
    }

    #[test]
    fn absent_layer_is_not_an_error() {
        let body = "---\ntype: T\ntitle: T\ndescription: D\n---\nbody\n";
        let diags = validate_yaml(body, "no-layer");
        assert!(
            diags.is_empty(),
            "absent layer should not be flagged, got: {diags:?}"
        );
    }

    // --- Controlled vocab: project ---

    #[test]
    fn bad_project_emits_error_at_project_locator() {
        let body = "---\ntype: T\ntitle: T\ndescription: D\nproject: unknown-project\n---\nbody\n";
        let diags = validate_yaml(body, "bad-project");
        assert_eq!(diags.len(), 1, "got: {diags:?}");
        assert_eq!(diags[0].severity, Severity::Error);
        assert_eq!(diags[0].locator, "project");
    }

    #[test]
    fn absent_project_is_not_an_error() {
        let body = "---\ntype: T\ntitle: T\ndescription: D\n---\nbody\n";
        let diags = validate_yaml(body, "no-project");
        assert!(
            diags.is_empty(),
            "absent project should not be flagged, got: {diags:?}"
        );
    }

    // --- Controlled vocab: status ---

    #[test]
    fn bad_status_emits_error_at_status_locator() {
        let body = "---\ntype: T\ntitle: T\ndescription: D\nstatus: pending\n---\nbody\n";
        let diags = validate_yaml(body, "bad-status");
        assert_eq!(diags.len(), 1, "got: {diags:?}");
        assert_eq!(diags[0].severity, Severity::Error);
        assert_eq!(diags[0].locator, "status");
    }

    #[test]
    fn absent_status_is_not_an_error() {
        let body = "---\ntype: T\ntitle: T\ndescription: D\n---\nbody\n";
        let diags = validate_yaml(body, "no-status");
        assert!(
            diags.is_empty(),
            "absent status should not be flagged, got: {diags:?}"
        );
    }

    // --- is_decision_id ---

    #[test]
    fn decision_id_accepts_bare_number() {
        assert!(is_decision_id("D7"));
        assert!(is_decision_id("D29"));
    }

    #[test]
    fn decision_id_accepts_full_form() {
        assert!(is_decision_id("D1-solo-practice"));
        assert!(is_decision_id("D15-okf-lowercase-doc-names"));
        assert!(is_decision_id("D29-mev-brain-validation-engine"));
    }

    #[test]
    fn decision_id_rejects_bare_d() {
        assert!(!is_decision_id("D"));
    }

    #[test]
    fn decision_id_rejects_missing_digit_after_d() {
        assert!(!is_decision_id("D-foo"));
    }

    #[test]
    fn decision_id_rejects_lowercase_d() {
        assert!(!is_decision_id("d15-foo"));
    }

    #[test]
    fn decision_id_rejects_uppercase_segment() {
        assert!(!is_decision_id("D15-Foo"));
    }

    // --- doc_id: kebab or decision-id ---

    #[test]
    fn non_kebab_doc_id_emits_error_at_doc_id_locator() {
        let body = "---\ntype: T\ntitle: T\ndescription: D\ndoc_id: My_Bad_Id\n---\nbody\n";
        let diags = validate_yaml(body, "bad-doc-id");
        assert_eq!(diags.len(), 1, "got: {diags:?}");
        assert_eq!(diags[0].severity, Severity::Error);
        assert_eq!(diags[0].locator, "doc_id");
    }

    #[test]
    fn valid_kebab_doc_id_is_clean() {
        let body = "---\ntype: T\ntitle: T\ndescription: D\ndoc_id: my-valid-id\n---\nbody\n";
        let diags = validate_yaml(body, "good-doc-id");
        assert!(
            diags.is_empty(),
            "valid doc_id should not be flagged, got: {diags:?}"
        );
    }

    #[test]
    fn decision_style_doc_id_is_clean() {
        let body = "---\ntype: T\ntitle: T\ndescription: D\ndoc_id: D15-okf-lowercase-doc-names\n---\nbody\n";
        let diags = validate_yaml(body, "decision-doc-id");
        assert!(
            diags.is_empty(),
            "decision-style doc_id should not be flagged, got: {diags:?}"
        );
    }

    // --- keywords count warning ---

    #[test]
    fn keywords_fewer_than_3_emits_warning() {
        let body = "---\ntype: T\ntitle: T\ndescription: D\nkeywords: [one, two]\n---\nbody\n";
        let diags = validate_yaml(body, "keywords-low");
        assert_eq!(diags.len(), 1, "got: {diags:?}");
        assert_eq!(diags[0].severity, Severity::Warning);
        assert_eq!(diags[0].locator, "keywords");
    }

    #[test]
    fn keywords_more_than_7_emits_warning() {
        let body = "---\ntype: T\ntitle: T\ndescription: D\nkeywords: [a, b, c, d, e, f, g, h]\n---\nbody\n";
        let diags = validate_yaml(body, "keywords-high");
        assert_eq!(diags.len(), 1, "got: {diags:?}");
        assert_eq!(diags[0].severity, Severity::Warning);
        assert_eq!(diags[0].locator, "keywords");
    }

    #[test]
    fn keywords_exactly_3_is_clean() {
        let body = "---\ntype: T\ntitle: T\ndescription: D\nkeywords: [a, b, c]\n---\nbody\n";
        let diags = validate_yaml(body, "keywords-3");
        assert!(
            diags.is_empty(),
            "3 keywords should be clean, got: {diags:?}"
        );
    }

    #[test]
    fn keywords_exactly_7_is_clean() {
        let body =
            "---\ntype: T\ntitle: T\ndescription: D\nkeywords: [a, b, c, d, e, f, g]\n---\nbody\n";
        let diags = validate_yaml(body, "keywords-7");
        assert!(
            diags.is_empty(),
            "7 keywords should be clean, got: {diags:?}"
        );
    }

    #[test]
    fn absent_keywords_is_not_flagged() {
        let body = "---\ntype: T\ntitle: T\ndescription: D\n---\nbody\n";
        let diags = validate_yaml(body, "no-keywords");
        assert!(
            diags.is_empty(),
            "absent keywords should not be flagged, got: {diags:?}"
        );
    }

    // --- Missing frontmatter ---

    #[test]
    fn missing_frontmatter_block_emits_single_error() {
        let body = "# Just a heading, no frontmatter\n";
        let diags = validate_yaml(body, "no-fm");
        assert_eq!(diags.len(), 1, "got: {diags:?}");
        assert_eq!(diags[0].severity, Severity::Error);
        assert_eq!(diags[0].locator, "frontmatter");
        assert!(diags[0].message.contains("missing or unterminated"));
    }

    // --- Malformed YAML ---

    #[test]
    fn malformed_yaml_emits_single_error() {
        let body = "---\ntitle: [unclosed bracket\n---\nbody\n";
        let diags = validate_yaml(body, "bad-yaml");
        assert_eq!(diags.len(), 1, "got: {diags:?}");
        assert_eq!(diags[0].severity, Severity::Error);
        assert_eq!(diags[0].locator, "frontmatter");
        assert!(diags[0].message.contains("malformed YAML"));
    }

    // --- Read failure ---

    #[test]
    fn unreadable_file_emits_single_error() {
        let cfg = full_test_config();
        let mf = MdFile {
            path: PathBuf::from("/nonexistent/mev/missing.md"),
            rel: PathBuf::from("missing.md"),
            stem: "missing".to_string(),
        };
        let diags = validate_md_file(&mf, &cfg);
        assert_eq!(diags.len(), 1, "got: {diags:?}");
        assert_eq!(diags[0].severity, Severity::Error);
        assert_eq!(diags[0].locator, "");
        assert!(diags[0].message.starts_with("could not read file:"));
    }

    // --- OKF exemption for root instruction files ---

    #[test]
    fn is_root_instruction_file_detects_readme() {
        let cfg = full_test_config();
        let mf = MdFile {
            path: PathBuf::from("/hq/README.md"),
            rel: PathBuf::from("README.md"),
            stem: "README".to_string(),
        };
        assert!(is_root_instruction_file(&mf, &cfg));
    }

    #[test]
    fn is_root_instruction_file_detects_claude_md() {
        let cfg = full_test_config();
        let mf = MdFile {
            path: PathBuf::from("/hq/CLAUDE.md"),
            rel: PathBuf::from("CLAUDE.md"),
            stem: "CLAUDE".to_string(),
        };
        assert!(is_root_instruction_file(&mf, &cfg));
    }

    #[test]
    fn is_root_instruction_file_false_for_ordinary_files() {
        let cfg = full_test_config();
        let mf = MdFile {
            path: PathBuf::from("/hq/planning/status.md"),
            rel: PathBuf::from("planning/status.md"),
            stem: "status".to_string(),
        };
        assert!(!is_root_instruction_file(&mf, &cfg));
    }

    #[test]
    fn is_root_instruction_file_false_for_deep_readme() {
        // docs/README.md is a corpus member (starts_with docs/) but NOT a root
        // instruction file — it must be validated for OKF, not exempt.
        let cfg = full_test_config();
        let mf = MdFile {
            path: PathBuf::from("/hq/docs/README.md"),
            rel: PathBuf::from("docs/README.md"),
            stem: "README".to_string(),
        };
        assert!(!is_root_instruction_file(&mf, &cfg));
    }

    #[test]
    fn root_readme_without_frontmatter_produces_no_diagnostic() {
        let dir = temp_dir("root-readme-exempt");
        // Write a README.md with no frontmatter.
        let path = dir.join("README.md");
        std::fs::write(&path, b"# Brain\nWelcome to the company brain.\n").unwrap();
        let mf = MdFile {
            path: path.clone(),
            rel: PathBuf::from("README.md"),
            stem: "README".to_string(),
        };
        let cfg = full_test_config();
        let diags = validate_md_file(&mf, &cfg);
        assert!(
            diags.is_empty(),
            "README.md without frontmatter should produce no diagnostic, got: {diags:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn root_claude_md_without_frontmatter_produces_no_diagnostic() {
        let dir = temp_dir("root-claude-exempt");
        let path = dir.join("CLAUDE.md");
        std::fs::write(&path, b"# CLAUDE\nProject instructions.\n").unwrap();
        let mf = MdFile {
            path: path.clone(),
            rel: PathBuf::from("CLAUDE.md"),
            stem: "CLAUDE".to_string(),
        };
        let cfg = full_test_config();
        let diags = validate_md_file(&mf, &cfg);
        assert!(
            diags.is_empty(),
            "CLAUDE.md without frontmatter should produce no diagnostic, got: {diags:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn root_readme_with_valid_frontmatter_is_validated_normally() {
        let dir = temp_dir("root-readme-with-fm");
        let content =
            "---\ntype: Index\ntitle: Brain README\ndescription: HQ entry point.\n---\n# Brain\n";
        let path = dir.join("README.md");
        std::fs::write(&path, content).unwrap();
        let mf = MdFile {
            path: path.clone(),
            rel: PathBuf::from("README.md"),
            stem: "README".to_string(),
        };
        let cfg = full_test_config();
        let diags = validate_md_file(&mf, &cfg);
        assert!(
            diags.is_empty(),
            "README.md with valid frontmatter should be clean, got: {diags:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn root_readme_with_invalid_frontmatter_is_flagged() {
        // A README.md that *does* carry frontmatter but is missing required fields
        // must still be validated — the exemption only applies to *absent* frontmatter.
        let dir = temp_dir("root-readme-bad-fm");
        let content = "---\ntitle: Brain README\n---\n# Brain\n";
        let path = dir.join("README.md");
        std::fs::write(&path, content).unwrap();
        let mf = MdFile {
            path: path.clone(),
            rel: PathBuf::from("README.md"),
            stem: "README".to_string(),
        };
        let cfg = full_test_config();
        let diags = validate_md_file(&mf, &cfg);
        // Missing `type` and `description` → at least 2 errors.
        assert!(
            !diags.is_empty(),
            "README.md with missing required fields must be flagged"
        );
        assert!(
            diags.iter().any(|d| d.locator == "type"),
            "expected 'type' error, got: {diags:?}"
        );
        assert!(
            diags.iter().any(|d| d.locator == "description"),
            "expected 'description' error, got: {diags:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
