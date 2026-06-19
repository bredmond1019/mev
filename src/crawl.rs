//! Content-tree crawl, classification, and filename-convention checks.
//!
//! This module implements Phase 1, Block B: walk the content root, classify each file
//! by its position in the tree and filename, build a [`Corpus`] for downstream validators
//! (Blocks C–E), and surface filename-convention violations as [`Diagnostic`]s.
//!
//! # Path derivation rule
//!
//! Given a path relative to the content root, classification proceeds left to right:
//! 1. First component must be `"paths"` — else skip (excludes `schemas/`, `shared/`, `*.md`).
//! 2. Second component is `path_id`.
//! 3. If the third component is `"pt-BR"`, `locale = PtBr` and it is consumed; else `locale = En`.
//! 4. Remaining tail:
//!    - `["metadata.json"]` → [`FileKind::PathMetadataJson`], `module_id = None`.
//!    - `["modules", file]` with `.json` extension → [`FileKind::LearnModuleJson`].
//!    - `["modules", file]` with `.mdx` extension → [`FileKind::ModuleMdx`].
//!    - anything else → skipped.

use std::path::{Path, PathBuf};

use crate::Diagnostic;

// ---------------------------------------------------------------------------
// Core enums
// ---------------------------------------------------------------------------

/// Locale of a content file. `En` is the default; `PtBr` nests under `pt-BR/`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Locale {
    En,
    PtBr,
}

/// Discriminates the three kinds of content file the content tree contains.
///
/// Non-content files (schemas, READMEs, dotfiles) are skipped during the walk and
/// never represented as a `ContentFile` — there is no `Unknown` variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    /// `paths/<path-id>/modules/NN-slug.json` (or pt-BR mirror)
    LearnModuleJson,
    /// `paths/<path-id>/metadata.json` (or pt-BR mirror)
    PathMetadataJson,
    /// `paths/<path-id>/modules/NN-slug.mdx` (or pt-BR mirror)
    ModuleMdx,
}

// ---------------------------------------------------------------------------
// ContentFile
// ---------------------------------------------------------------------------

/// A single classified content file from the content tree.
#[derive(Debug, Clone)]
pub struct ContentFile {
    /// Absolute path as walked (joined on the content root).
    pub path: PathBuf,
    /// Path relative to the content root — used for diagnostic locators and display.
    pub rel: PathBuf,
    /// Which kind of content file this is.
    pub kind: FileKind,
    /// The path-section slug (second component of the relative path under `paths/`).
    pub path_id: String,
    /// Module stem (`None` for `PathMetadataJson`; `Some("NN-slug")` for json/mdx modules).
    pub module_id: Option<String>,
    /// Locale this file belongs to.
    pub locale: Locale,
}

// ---------------------------------------------------------------------------
// Corpus
// ---------------------------------------------------------------------------

/// All content files collected from a crawl, with accessors for downstream validators.
///
/// Iteration order is deterministic: [`crawl`] inserts files in `walkdir` order, and the
/// accessors sort or deduplicate using [`std::collections::BTreeSet`] where needed.
#[derive(Debug, Default)]
pub struct Corpus {
    /// All classified content files in walk order.
    pub files: Vec<ContentFile>,
}

impl Corpus {
    /// Every distinct `path_id` present in the corpus, sorted alphabetically.
    pub fn path_ids(&self) -> Vec<&str> {
        let mut ids: Vec<&str> = self
            .files
            .iter()
            .map(|f| f.path_id.as_str())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        ids.sort_unstable();
        ids
    }

    /// All content files for a given `path_id` and `locale` that represent modules
    /// (i.e. have a `module_id` — excludes `PathMetadataJson`).
    pub fn modules_for<'a>(&'a self, path_id: &str, locale: Locale) -> Vec<&'a ContentFile> {
        self.files
            .iter()
            .filter(|f| f.path_id == path_id && f.locale == locale && f.module_id.is_some())
            .collect()
    }

    /// Look up a single content file by `(path_id, module_id, locale, kind)`.
    ///
    /// Used by Block D to pair `.json` ↔ `.mdx` module files.
    pub fn get(
        &self,
        path_id: &str,
        module_id: &str,
        locale: Locale,
        kind: FileKind,
    ) -> Option<&ContentFile> {
        self.files.iter().find(|f| {
            f.path_id == path_id
                && f.module_id.as_deref() == Some(module_id)
                && f.locale == locale
                && f.kind == kind
        })
    }
}

// ---------------------------------------------------------------------------
// Classification
// ---------------------------------------------------------------------------

/// Classify a single filesystem path relative to `root`.
///
/// Returns `None` for any path that is not a recognised content file — the caller simply
/// skips it, so non-content files produce no diagnostics here.
fn classify(root: &Path, path: &Path) -> Option<ContentFile> {
    let rel = path.strip_prefix(root).ok()?;

    // Collect UTF-8 components; bail on non-UTF-8 (very rare on these repos).
    let comps: Vec<&str> = rel
        .components()
        .map(|c| c.as_os_str().to_str())
        .collect::<Option<Vec<_>>>()?;

    // First component must be "paths".
    if comps.first()? != &"paths" {
        return None;
    }

    // Second component is the path-id.
    let path_id = comps.get(1)?.to_string();

    // Determine locale and where the tail starts.
    let (locale, tail_start) = if comps.get(2) == Some(&"pt-BR") {
        (Locale::PtBr, 3usize)
    } else {
        (Locale::En, 2usize)
    };

    let tail = &comps[tail_start..];

    let (kind, module_id) = match tail {
        ["metadata.json"] => (FileKind::PathMetadataJson, None),
        ["modules", file] if file.ends_with(".json") => {
            let stem = Path::new(file).file_stem()?.to_str()?.to_string();
            (FileKind::LearnModuleJson, Some(stem))
        }
        ["modules", file] if file.ends_with(".mdx") => {
            let stem = Path::new(file).file_stem()?.to_str()?.to_string();
            (FileKind::ModuleMdx, Some(stem))
        }
        _ => return None,
    };

    Some(ContentFile {
        path: path.to_path_buf(),
        rel: rel.to_path_buf(),
        kind,
        path_id,
        module_id,
        locale,
    })
}

// ---------------------------------------------------------------------------
// Filename-convention checks
// ---------------------------------------------------------------------------

/// Check a classified file's filename against the naming conventions and return any violations.
///
/// A violation is an [`Severity::Error`] diagnostic (mirrors the TS validator's `validateFileName`
/// behaviour). The file is still added to the corpus — a bad name does not drop the file.
fn check_filename(cf: &ContentFile) -> Vec<Diagnostic> {
    let mut diags = Vec::new();

    let name = match cf.rel.file_name().and_then(|n| n.to_str()) {
        Some(n) => n,
        None => return diags,
    };

    // No spaces.
    if name.contains(' ') {
        diags.push(Diagnostic::error(
            cf.rel.clone(),
            "",
            format!("filename contains spaces: {name}"),
        ));
    }

    // Must be lowercase.
    if name != name.to_lowercase() {
        diags.push(Diagnostic::error(
            cf.rel.clone(),
            "",
            format!("filename must be lowercase: {name}"),
        ));
    }

    // Module files must match `NN-slug.(json|mdx)`.
    if matches!(cf.kind, FileKind::LearnModuleJson | FileKind::ModuleMdx)
        && !is_valid_module_filename(name)
    {
        diags.push(Diagnostic::error(
            cf.rel.clone(),
            "",
            format!("module filename must match NN-slug.(json|mdx): {name}"),
        ));
    }

    diags
}

/// Return `true` if `name` matches `^\d{2}-[a-z0-9-]+\.(json|mdx)$` without the `regex` crate.
fn is_valid_module_filename(name: &str) -> bool {
    // Split off the extension.
    let (stem, ext) = match name.rsplit_once('.') {
        Some(pair) => pair,
        None => return false,
    };

    if ext != "json" && ext != "mdx" {
        return false;
    }

    // Stem must be `NN-slug` where NN is exactly two ASCII digits.
    let (prefix, slug) = match stem.split_once('-') {
        Some(pair) => pair,
        None => return false,
    };

    if prefix.len() != 2 || !prefix.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }

    // Slug must be non-empty and contain only `[a-z0-9-]`.
    !slug.is_empty()
        && slug
            .chars()
            .all(|c| matches!(c, 'a'..='z' | '0'..='9' | '-'))
}

// ---------------------------------------------------------------------------
// Public walk entry point
// ---------------------------------------------------------------------------

/// Walk `root`, classify every content file, collect filename diagnostics, and build a [`Corpus`].
///
/// IO/walk errors are surfaced as [`Diagnostic`]s rather than propagating as `Err` — the walk
/// continues past inaccessible entries.
pub fn crawl(root: &Path) -> (Corpus, Vec<Diagnostic>) {
    let mut corpus = Corpus::default();
    let mut diags = Vec::new();

    for entry in walkdir::WalkDir::new(root) {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                diags.push(Diagnostic::error(root, "", format!("walk error: {e}")));
                continue;
            }
        };

        if !entry.file_type().is_file() {
            continue;
        }

        if let Some(cf) = classify(root, entry.path()) {
            diags.extend(check_filename(&cf));
            corpus.files.push(cf);
        }
    }

    (corpus, diags)
}

// ---------------------------------------------------------------------------
// Unit tests for types and helpers
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_file(
        path_id: &str,
        module_id: Option<&str>,
        locale: Locale,
        kind: FileKind,
    ) -> ContentFile {
        ContentFile {
            path: PathBuf::from("/root/paths/demo"),
            rel: PathBuf::from("paths/demo/metadata.json"),
            kind,
            path_id: path_id.to_string(),
            module_id: module_id.map(|s| s.to_string()),
            locale,
        }
    }

    // --- FileKind / Locale derive tests ---

    #[test]
    fn file_kind_eq() {
        assert_eq!(FileKind::LearnModuleJson, FileKind::LearnModuleJson);
        assert_ne!(FileKind::LearnModuleJson, FileKind::ModuleMdx);
    }

    #[test]
    fn locale_eq() {
        assert_eq!(Locale::En, Locale::En);
        assert_ne!(Locale::En, Locale::PtBr);
    }

    // --- Corpus accessors ---

    #[test]
    fn path_ids_sorted_and_deduped() {
        let mut corpus = Corpus::default();
        corpus.files.push(make_file(
            "beta",
            None,
            Locale::En,
            FileKind::PathMetadataJson,
        ));
        corpus.files.push(make_file(
            "alpha",
            Some("01-intro"),
            Locale::En,
            FileKind::LearnModuleJson,
        ));
        corpus.files.push(make_file(
            "alpha",
            Some("01-intro"),
            Locale::En,
            FileKind::ModuleMdx,
        ));

        let ids = corpus.path_ids();
        assert_eq!(ids, vec!["alpha", "beta"]);
    }

    #[test]
    fn modules_for_filters_by_locale_and_excludes_metadata() {
        let mut corpus = Corpus::default();
        corpus.files.push(make_file(
            "demo",
            None,
            Locale::En,
            FileKind::PathMetadataJson,
        ));
        corpus.files.push(make_file(
            "demo",
            Some("01-x"),
            Locale::En,
            FileKind::LearnModuleJson,
        ));
        corpus.files.push(make_file(
            "demo",
            Some("01-x"),
            Locale::PtBr,
            FileKind::ModuleMdx,
        ));

        let en = corpus.modules_for("demo", Locale::En);
        assert_eq!(en.len(), 1);
        assert_eq!(en[0].module_id.as_deref(), Some("01-x"));

        let ptbr = corpus.modules_for("demo", Locale::PtBr);
        assert_eq!(ptbr.len(), 1);
        assert_eq!(ptbr[0].kind, FileKind::ModuleMdx);
    }

    #[test]
    fn get_finds_exact_match() {
        let mut corpus = Corpus::default();
        corpus.files.push(make_file(
            "demo",
            Some("01-x"),
            Locale::En,
            FileKind::LearnModuleJson,
        ));
        corpus.files.push(make_file(
            "demo",
            Some("01-x"),
            Locale::En,
            FileKind::ModuleMdx,
        ));

        assert!(
            corpus
                .get("demo", "01-x", Locale::En, FileKind::LearnModuleJson)
                .is_some()
        );
        assert!(
            corpus
                .get("demo", "01-x", Locale::En, FileKind::ModuleMdx)
                .is_some()
        );
        assert!(
            corpus
                .get("demo", "01-x", Locale::PtBr, FileKind::ModuleMdx)
                .is_none()
        );
        assert!(
            corpus
                .get("other", "01-x", Locale::En, FileKind::ModuleMdx)
                .is_none()
        );
    }

    // --- is_valid_module_filename ---

    #[test]
    fn valid_module_filenames() {
        assert!(is_valid_module_filename("01-intro.json"));
        assert!(is_valid_module_filename("99-some-long-slug.mdx"));
        assert!(is_valid_module_filename("10-abc123.json"));
    }

    #[test]
    fn invalid_module_filenames() {
        assert!(!is_valid_module_filename("1-intro.json")); // single digit prefix
        assert!(!is_valid_module_filename("ab-intro.json")); // non-digit prefix
        assert!(!is_valid_module_filename("01-Intro.json")); // uppercase in slug
        assert!(!is_valid_module_filename("intro.json")); // no prefix at all
        assert!(!is_valid_module_filename("01-intro.txt")); // wrong extension
        assert!(!is_valid_module_filename("01-.json")); // empty slug
        assert!(!is_valid_module_filename("01-intro_x.json")); // underscore not allowed
    }
}
