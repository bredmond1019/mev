//! Brain-repo Markdown crawl.
//!
//! Walks a directory tree collecting every `*.md` file, applying a two-layer skip-list:
//!
//! **Name blocklist** — any directory whose name appears in the caller-supplied `skip_dirs`
//! slice (sourced from `brain.toml`'s `[crawl].skip_dirs`) is pruned.
//!
//! **Nested-git rule** — any directory at `depth() > 0` that contains its own `.git` entry
//! is pruned.  The `depth() > 0` guard exempts the brain root itself, which is a git repo
//! but must not be pruned.
//!
//! [`crawl_brain`] returns `(Vec<MdFile>, Vec<Diagnostic>)` mirroring the learn-ai crawl
//! shape.  Walk/IO errors are surfaced as [`Diagnostic`]s rather than propagated as `Err`.
//!
//! [`crawl_corpus`] returns `(Corpus, Vec<Diagnostic>)` — an owned, serializable multi-root
//! corpus result (the manifest seed for Phase 3B Block Q), separate from diagnostics.

use std::path::{Path, PathBuf};

use crate::Diagnostic;
use crate::brain::config::BrainConfig;
use crate::brain::okf::OkfFrontmatter;
use crate::shared::extract_frontmatter;

// ---------------------------------------------------------------------------
// MdFile
// ---------------------------------------------------------------------------

/// A single Markdown file found during a brain-repo crawl.
#[derive(Debug, Clone)]
pub struct MdFile {
    /// Absolute path as walked.
    pub path: PathBuf,
    /// Path relative to the crawl root — used for diagnostic locators and display.
    pub rel: PathBuf,
    /// File stem, e.g. `"status"` for `status.md`.
    pub stem: String,
}

// ---------------------------------------------------------------------------
// Corpus types (D4 forward-compat — owned, serializable manifest seed)
// ---------------------------------------------------------------------------

/// A single file entry in the Brain corpus.
///
/// `scope` is the stable slug of the owning registry unit (from `brain.toml`'s
/// `[[repos]]` entries), resolved once at crawl time so the graph block and the
/// Phase 3B Block Q manifest emitter both get it without re-crawling.
///
/// `metadata` holds the parsed OKF frontmatter from the file, extracted once
/// during the crawl (D5 extract-once refactor).  `None` if the file has no
/// frontmatter or the frontmatter cannot be parsed.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CorpusEntry {
    /// Absolute path as walked.
    pub path: PathBuf,
    /// Path relative to the HQ crawl root.
    pub rel: PathBuf,
    /// File stem, e.g. `"status"` for `status.md`.
    pub stem: String,
    /// Stable slug of the owning scope unit (e.g. `"brain"`, `"mev"`, `"core"`).
    pub scope: String,
    /// Parsed OKF frontmatter, extracted once at crawl time (D5 extract-once).
    /// `None` if the file has no frontmatter or the YAML cannot be parsed.
    pub metadata: Option<OkfFrontmatter>,
}

/// The complete Brain corpus: every `.md` file that belongs to the multi-root
/// crawl, each carrying its resolved owning-unit `scope`.
///
/// Derives [`serde::Serialize`] so Phase 3B Block Q can emit it as a manifest
/// without re-crawling — the single corpus definition consumed by both the
/// OKF validator and the graph/embedder.
#[derive(Debug, Default, serde::Serialize)]
pub struct Corpus {
    pub entries: Vec<CorpusEntry>,
}

// ---------------------------------------------------------------------------
// Pruning helpers
// ---------------------------------------------------------------------------

/// Return `true` if a directory should be skipped based on the caller-supplied `skip_dirs` slice.
///
/// The slice is sourced from `brain.toml`'s `[crawl].skip_dirs` — no names are hardcoded here.
///
/// Two matching modes are supported:
/// - **Name match** — a simple name like `"target"` matches any directory whose leaf name equals it.
/// - **Path match** — a path like `"planning/archive"` matches a directory whose path relative to
///   `root` equals that entry (using platform path separators).  `rel` must be the directory's path
///   relative to the crawl root; pass `None` to skip path-style matching (name-only mode).
pub(crate) fn is_blocklisted_name(name: &str, rel: Option<&Path>, skip_dirs: &[String]) -> bool {
    skip_dirs.iter().any(|d| {
        // Name-only entry: matches the directory leaf name.
        if !d.contains('/') && !d.contains(std::path::MAIN_SEPARATOR) {
            return d == name;
        }
        // Path-style entry: matches the relative path from root (normalised to platform separators).
        if let Some(rel_path) = rel {
            let d_path = Path::new(d);
            return rel_path == d_path;
        }
        false
    })
}

/// Return `true` if a file name is on the file blocklist.
///
/// Blocklisted files: other-tool config files (`CLAUDE.local.md`, `GEMINI.md`) and
/// transient session artifacts (`handoff.md`) that are never OKF docs.
///
/// Note: `CLAUDE.md` is **not** blocklisted — it is a valid corpus root-instruction file
/// included as a corpus leaf when present at a unit's root level.
pub(crate) fn is_blocklisted_file(name: &str) -> bool {
    matches!(name, "CLAUDE.local.md" | "GEMINI.md" | "handoff.md")
}

/// Return `true` if `dir_path` contains a `.git` entry (file or directory).
///
/// Used to detect nested sub-project repos.  Only called for directories at `depth() > 0`
/// so the brain root (which is itself a git repo) is always exempt.
pub(crate) fn has_nested_git(dir_path: &Path) -> bool {
    dir_path.join(".git").exists()
}

// ---------------------------------------------------------------------------
// Public walk entry point
// ---------------------------------------------------------------------------

/// Walk `root`, collect every `*.md` file, and return an [`MdFile`] list plus any
/// walk-error diagnostics.
///
/// `skip_dirs` is the list of directory names to prune (sourced from
/// `brain.toml`'s `[crawl].skip_dirs` via [`BrainConfig`][crate::brain::config::BrainConfig]).
///
/// Pruning rules (applied at the directory level so entire subtrees are skipped):
/// - Any directory whose name is in `skip_dirs`.
/// - Any directory at depth > 0 that contains its own `.git` entry.
pub fn crawl_brain(root: &Path, skip_dirs: &[String]) -> (Vec<MdFile>, Vec<Diagnostic>) {
    let mut files = Vec::new();
    let mut diags = Vec::new();

    let iter = walkdir::WalkDir::new(root).into_iter().filter_entry(|e| {
        // Always allow the root itself (depth 0).
        if e.depth() == 0 {
            return true;
        }

        // For directories: apply the two-layer skip-list.
        if e.file_type().is_dir() {
            let name = e.file_name().to_string_lossy();
            // Compute relative path for path-style skip_dirs entries (e.g. "planning/archive").
            let rel = e.path().strip_prefix(root).ok();
            if is_blocklisted_name(&name, rel, skip_dirs) {
                return false;
            }
            // Nested-git rule: prune any sub-directory that is itself a git repo.
            if has_nested_git(e.path()) {
                return false;
            }
        }

        true
    });

    for entry in iter {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                diags.push(Diagnostic::error(root, "", format!("walk error: {e}")));
                continue;
            }
        };

        // Only collect files (not directories).
        if !entry.file_type().is_file() {
            continue;
        }

        // Only collect `.md` files.
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }

        // Skip blocklisted file names.
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if is_blocklisted_file(file_name) {
            continue;
        }

        let rel = match path.strip_prefix(root) {
            Ok(r) => r.to_path_buf(),
            Err(_) => {
                diags.push(Diagnostic::error(
                    path,
                    "",
                    "could not compute relative path".to_string(),
                ));
                continue;
            }
        };

        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();

        files.push(MdFile {
            path: path.to_path_buf(),
            rel,
            stem,
        });
    }

    (files, diags)
}

// ---------------------------------------------------------------------------
// Corpus crawl helpers
// ---------------------------------------------------------------------------

/// Compute a file's path relative to its owning unit's root.
///
/// `rel` is the file's path relative to the HQ crawl root.
/// `unit_repo_path` is the owning unit's `repo_path` from `brain.toml` (e.g. `"core/mev"`
/// or `"."` for the HQ root unit).
///
/// For the root unit (`"."` or `""`), the unit root equals the HQ root, so `rel` is
/// returned unchanged.  For all other units, `rel.strip_prefix(unit_repo_path)` gives the
/// path relative to that unit's root.  Returns `None` on an unexpected prefix mismatch.
fn rel_to_unit_root<'a>(rel: &'a Path, unit_repo_path: &str) -> Option<&'a Path> {
    let trimmed = unit_repo_path.trim();
    if trimmed == "." || trimmed.is_empty() {
        return Some(rel);
    }
    rel.strip_prefix(trimmed).ok()
}

/// Return `true` if a file (given its path relative to its owning unit's root) is a member
/// of that unit's corpus.
///
/// Corpus membership rules (per the canonical spec decision):
/// - The file is the unit-root `README.md` or `CLAUDE.md` (exactly at the unit root level).
/// - The file lives under the unit's `planning/` subtree.
/// - The file lives under the unit's `docs/` subtree.
///
/// Everything else — `src/*.md`, stray root-level `.md`, files under unregistered dirs —
/// is excluded.
pub(crate) fn is_corpus_member(rel_to_unit: &Path) -> bool {
    // Root instruction files (exactly at the unit root).
    if rel_to_unit == Path::new("README.md") || rel_to_unit == Path::new("CLAUDE.md") {
        return true;
    }
    // Under planning/ or docs/ (any depth).
    if rel_to_unit.starts_with("planning") || rel_to_unit.starts_with("docs") {
        return true;
    }
    false
}

/// Return `true` if a file should be excluded as ephemeral or underscore-prefixed.
///
/// - `handoff.md` — transient session artifact, never a stable corpus member.
/// - Any file whose **name** starts with `_` — draft/working files excluded by convention.
pub(crate) fn is_ephemeral(file_name: &str) -> bool {
    matches!(file_name, "handoff.md" | "tasks.md" | "breakdown.md" | "worklog.md") || file_name.starts_with('_')
}

/// Walk `root` and return an owned, serializable [`Corpus`] plus any walk-error diagnostics.
///
/// This is the canonical multi-root Brain corpus crawl (D4 forward-compat).  It differs from
/// [`crawl_brain`] in several ways:
///
/// 1. **Unit-ownership bounds membership** — no nested-git pruning; every `.md` file is tested
///    against the registry-driven membership rule instead.
/// 2. **Scope-resolved entries** — each [`CorpusEntry`] carries the stable slug of its owning
///    registry unit, resolved once here via [`crate::brain::scope::owning_unit`].
/// 3. **Membership rule** — a file is included iff its path relative to its owning unit is
///    under `planning/` or `docs/`, or it is the unit-root `README.md`/`CLAUDE.md`.
/// 4. **Ephemeral exclusion** — `handoff.md` and `_`-prefixed files are dropped.
/// 5. **Separate corpus + diagnostics** — the [`Corpus`] is returned as a first-class owned
///    value so callers (graph block, manifest emitter) can reuse it without re-crawling.
pub fn crawl_corpus(root: &Path, config: &BrainConfig) -> (Corpus, Vec<Diagnostic>) {
    let mut entries = Vec::new();
    let mut diags = Vec::new();

    let skip_dirs = &config.crawl.skip_dirs;

    let iter = walkdir::WalkDir::new(root).into_iter().filter_entry(|e| {
        // Always allow the root itself (depth 0).
        if e.depth() == 0 {
            return true;
        }

        // For directories: prune by skip_dirs (name-mode only — bare component at any depth).
        if e.file_type().is_dir() {
            let name = e.file_name().to_string_lossy();
            // Compute relative path for path-style skip_dirs entries.
            let rel = e.path().strip_prefix(root).ok();
            if is_blocklisted_name(&name, rel, skip_dirs) {
                return false;
            }
            // No nested-git pruning: unit-ownership bounds membership.
        }

        true
    });

    for entry in iter {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                diags.push(Diagnostic::error(root, "", format!("walk error: {e}")));
                continue;
            }
        };

        // Only collect files (not directories).
        if !entry.file_type().is_file() {
            continue;
        }

        // Only collect `.md` files.
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }

        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

        // Drop ephemeral files (handoff.md, _-prefixed) before the membership test.
        if is_ephemeral(file_name) {
            continue;
        }

        // Compute path relative to the HQ crawl root.
        let rel = match path.strip_prefix(root) {
            Ok(r) => r.to_path_buf(),
            Err(_) => {
                diags.push(Diagnostic::error(
                    path,
                    "",
                    "could not compute relative path".to_string(),
                ));
                continue;
            }
        };

        // Resolve owning unit and compute the file's path relative to that unit's root.
        let (scope, unit_repo_path) = crate::brain::scope::owning_unit(&rel, config);
        let rel_unit = match rel_to_unit_root(&rel, &unit_repo_path) {
            Some(r) => r,
            None => {
                // Unexpected prefix mismatch — emit a diagnostic and skip.
                diags.push(Diagnostic::error(
                    path,
                    "",
                    format!(
                        "could not compute unit-relative path (unit repo_path '{unit_repo_path}')"
                    ),
                ));
                continue;
            }
        };

        // Apply corpus membership rule.
        if !is_corpus_member(rel_unit) {
            continue;
        }

        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();

        // D5 extract-once: parse frontmatter once here; callers (graph, manifest) use
        // entry.metadata directly and never re-read the file for frontmatter.
        let metadata = std::fs::read_to_string(path).ok().and_then(|contents| {
            let yaml = extract_frontmatter(&contents)?;
            serde_yaml::from_str::<OkfFrontmatter>(yaml).ok()
        });

        entries.push(CorpusEntry {
            path: path.to_path_buf(),
            rel,
            stem,
            scope,
            metadata,
        });
    }

    (Corpus { entries }, diags)
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- is_blocklisted_name ---

    fn standard_skip_dirs() -> Vec<String> {
        [
            "target",
            "node_modules",
            ".git",
            ".claude",
            ".repo-backups",
            ".agent",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }

    #[test]
    fn blocklisted_names_are_rejected() {
        let skip = standard_skip_dirs();
        assert!(is_blocklisted_name("target", None, &skip));
        assert!(is_blocklisted_name("node_modules", None, &skip));
        assert!(is_blocklisted_name(".git", None, &skip));
        assert!(is_blocklisted_name(".claude", None, &skip));
        assert!(is_blocklisted_name(".repo-backups", None, &skip));
        assert!(is_blocklisted_name(".agent", None, &skip));
    }

    #[test]
    fn ordinary_names_are_allowed() {
        let skip = standard_skip_dirs();
        assert!(!is_blocklisted_name("docs", None, &skip));
        assert!(!is_blocklisted_name("src", None, &skip));
        assert!(!is_blocklisted_name("planning", None, &skip));
        assert!(!is_blocklisted_name("README.md", None, &skip));
        assert!(!is_blocklisted_name("target-extra", None, &skip)); // prefix match must not fire
    }

    #[test]
    fn empty_skip_dirs_allows_all_names() {
        let skip: Vec<String> = vec![];
        assert!(!is_blocklisted_name("target", None, &skip));
        assert!(!is_blocklisted_name(".git", None, &skip));
        assert!(!is_blocklisted_name("node_modules", None, &skip));
    }

    #[test]
    fn custom_skip_dirs_are_respected() {
        let skip = vec!["custom-dir".to_string(), "another".to_string()];
        assert!(is_blocklisted_name("custom-dir", None, &skip));
        assert!(is_blocklisted_name("another", None, &skip));
        assert!(!is_blocklisted_name("target", None, &skip));
    }

    #[test]
    fn path_style_skip_dirs_match_relative_path() {
        let skip = vec!["planning/archive".to_string()];
        // "archive" directory at relative path "planning/archive" should be blocked.
        let rel = Path::new("planning/archive");
        assert!(is_blocklisted_name("archive", Some(rel), &skip));
        // "archive" directory at a different relative path should not be blocked.
        let rel2 = Path::new("docs/archive");
        assert!(!is_blocklisted_name("archive", Some(rel2), &skip));
        // "planning" directory itself should not be blocked by a path-style entry.
        let rel3 = Path::new("planning");
        assert!(!is_blocklisted_name("planning", Some(rel3), &skip));
    }

    #[test]
    fn path_style_skip_dirs_without_rel_do_not_match() {
        let skip = vec!["planning/archive".to_string()];
        // With no rel (None), path-style entries are skipped — name won't match because
        // the entry contains a separator.
        assert!(!is_blocklisted_name("archive", None, &skip));
    }

    // --- is_blocklisted_file ---

    #[test]
    fn blocklisted_files_are_rejected() {
        // CLAUDE.md is no longer blocklisted — it is a corpus root-instruction file.
        assert!(is_blocklisted_file("CLAUDE.local.md"));
        assert!(is_blocklisted_file("GEMINI.md"));
        assert!(is_blocklisted_file("handoff.md"));
    }

    #[test]
    fn ordinary_files_are_allowed() {
        assert!(!is_blocklisted_file("status.md"));
        assert!(!is_blocklisted_file("README.md"));
        assert!(!is_blocklisted_file("CLAUDE.md")); // no longer blocklisted
        assert!(!is_blocklisted_file("context.md"));
        assert!(!is_blocklisted_file("claude-notes.md")); // prefix match must not fire
    }

    // --- is_corpus_member ---

    #[test]
    fn readme_at_unit_root_is_corpus_member() {
        assert!(is_corpus_member(Path::new("README.md")));
    }

    #[test]
    fn claude_md_at_unit_root_is_corpus_member() {
        assert!(is_corpus_member(Path::new("CLAUDE.md")));
    }

    #[test]
    fn planning_file_is_corpus_member() {
        assert!(is_corpus_member(Path::new("planning/status.md")));
        assert!(is_corpus_member(Path::new("planning/decisions/D1-foo.md")));
    }

    #[test]
    fn docs_file_is_corpus_member() {
        assert!(is_corpus_member(Path::new("docs/index.md")));
        assert!(is_corpus_member(Path::new("docs/projects/mev.md")));
    }

    #[test]
    fn src_file_is_not_corpus_member() {
        assert!(!is_corpus_member(Path::new("src/lib.md")));
        assert!(!is_corpus_member(Path::new("src/brain/notes.md")));
    }

    #[test]
    fn stray_root_md_is_not_corpus_member() {
        // A .md at the unit root that is not README.md or CLAUDE.md is excluded.
        assert!(!is_corpus_member(Path::new("NOTES.md")));
        assert!(!is_corpus_member(Path::new("random.md")));
    }

    #[test]
    fn file_under_unregistered_nested_dir_is_not_corpus_member() {
        // e.g. a file under a subdirectory that is neither planning/ nor docs/
        assert!(!is_corpus_member(Path::new("assets/image.md")));
        assert!(!is_corpus_member(Path::new("tests/fixture.md")));
    }

    // --- is_ephemeral ---

    #[test]
    fn handoff_md_is_ephemeral() {
        assert!(is_ephemeral("handoff.md"));
    }

    #[test]
    fn underscore_prefixed_is_ephemeral() {
        assert!(is_ephemeral("_draft.md"));
        assert!(is_ephemeral("_working.md"));
    }

    #[test]
    fn ordinary_files_are_not_ephemeral() {
        assert!(!is_ephemeral("README.md"));
        assert!(!is_ephemeral("status.md"));
        assert!(!is_ephemeral("CLAUDE.md"));
    }

    // --- rel_to_unit_root ---

    #[test]
    fn rel_to_unit_root_for_root_unit() {
        let rel = Path::new("planning/status.md");
        assert_eq!(rel_to_unit_root(rel, "."), Some(rel));
        assert_eq!(rel_to_unit_root(rel, ""), Some(rel));
    }

    #[test]
    fn rel_to_unit_root_strips_prefix() {
        let rel = Path::new("core/mev/planning/status.md");
        let expected = Path::new("planning/status.md");
        assert_eq!(rel_to_unit_root(rel, "core/mev"), Some(expected));
    }

    #[test]
    fn rel_to_unit_root_bad_prefix_returns_none() {
        let rel = Path::new("core/other/planning/status.md");
        assert!(rel_to_unit_root(rel, "core/mev").is_none());
    }

    // --- crawl_corpus serialization roundtrip ---

    #[test]
    fn corpus_is_serializable_to_json() {
        // Build a minimal in-memory corpus entry and verify serde_json serializes it,
        // including the metadata field (None → null in JSON).
        let entry = CorpusEntry {
            path: std::path::PathBuf::from("/hq/planning/status.md"),
            rel: std::path::PathBuf::from("planning/status.md"),
            stem: "status".to_string(),
            scope: "brain".to_string(),
            metadata: None,
        };
        let corpus = Corpus {
            entries: vec![entry],
        };
        let json = serde_json::to_string(&corpus).expect("corpus must serialize to JSON");
        assert!(json.contains("\"scope\":\"brain\""));
        assert!(json.contains("\"stem\":\"status\""));
        assert!(json.contains("\"entries\""));
        // metadata field must be present in serialized output (null when None).
        assert!(
            json.contains("\"metadata\""),
            "metadata field must appear in JSON"
        );
    }

    #[test]
    fn corpus_entry_with_frontmatter_carries_metadata() {
        let dir = std::env::temp_dir().join("mev-corpus-metadata-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("planning")).unwrap();
        // Write a file with valid OKF frontmatter.
        std::fs::write(
            dir.join("planning/status.md"),
            "---\ntype: ProjectStatus\ntitle: MEV Status\ndescription: Current state.\ndoc_id: mev-status\n---\n# body\n",
        ).unwrap();
        // Write a file with no frontmatter.
        std::fs::write(dir.join("README.md"), b"# README\nNo frontmatter here.\n").unwrap();

        let cfg = brain_only_config();
        let (corpus, diags) = crawl_corpus(&dir, &cfg);
        assert!(diags.is_empty(), "expected no diagnostics: {diags:?}");

        // Find the entry for planning/status.md — it should carry Some(metadata).
        let status_entry = corpus
            .entries
            .iter()
            .find(|e| e.stem == "status")
            .expect("planning/status.md must be in corpus");
        assert!(
            status_entry.metadata.is_some(),
            "entry with valid frontmatter must carry Some(metadata)"
        );
        let meta = status_entry.metadata.as_ref().unwrap();
        assert_eq!(
            meta.type_.as_deref(),
            Some("ProjectStatus"),
            "type_ must match frontmatter"
        );
        assert_eq!(
            meta.title.as_deref(),
            Some("MEV Status"),
            "title must match frontmatter"
        );
        assert_eq!(
            meta.doc_id.as_deref(),
            Some("mev-status"),
            "doc_id must match frontmatter"
        );

        // Find the entry for README.md — it should carry None.
        let readme_entry = corpus
            .entries
            .iter()
            .find(|e| e.stem == "README")
            .expect("README.md must be in corpus");
        assert!(
            readme_entry.metadata.is_none(),
            "entry without frontmatter must carry None metadata"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- crawl_corpus end-to-end walk ---

    fn brain_only_config() -> crate::brain::config::BrainConfig {
        use crate::brain::config::{BrainConfig, CrawlConfig, RepoEntry, VocabConfig};
        BrainConfig {
            vocab: VocabConfig::default(),
            crawl: CrawlConfig {
                skip_dirs: vec![
                    "target".to_string(),
                    "node_modules".to_string(),
                    ".git".to_string(),
                    "archive".to_string(),
                    "trees".to_string(),
                ],
            },
            repos: vec![RepoEntry {
                slug: "brain".to_string(),
                tier: "primary".to_string(),
                repo_path: ".".to_string(),
                status_file: String::new(),
                cache_doc: String::new(),
                heading: String::new(),
            }],
        }
    }

    #[test]
    fn corpus_includes_planning_and_docs_and_root_files() {
        let dir = std::env::temp_dir().join("mev-corpus-basic");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("planning")).unwrap();
        std::fs::create_dir_all(dir.join("docs")).unwrap();
        std::fs::write(dir.join("README.md"), b"").unwrap();
        std::fs::write(dir.join("CLAUDE.md"), b"").unwrap();
        std::fs::write(dir.join("planning/status.md"), b"").unwrap();
        std::fs::write(dir.join("docs/index.md"), b"").unwrap();

        let cfg = brain_only_config();
        let (corpus, diags) = crawl_corpus(&dir, &cfg);
        assert!(diags.is_empty(), "expected no diagnostics: {diags:?}");

        let stems: Vec<&str> = corpus.entries.iter().map(|e| e.stem.as_str()).collect();
        assert!(stems.contains(&"README"), "expected README.md in corpus");
        assert!(stems.contains(&"CLAUDE"), "expected CLAUDE.md in corpus");
        assert!(
            stems.contains(&"status"),
            "expected planning/status.md in corpus"
        );
        assert!(stems.contains(&"index"), "expected docs/index.md in corpus");
        assert_eq!(
            corpus.entries.len(),
            4,
            "expected exactly 4 entries, got: {stems:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corpus_excludes_src_and_stray_root_md() {
        let dir = std::env::temp_dir().join("mev-corpus-excludes");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::create_dir_all(dir.join("planning")).unwrap();
        std::fs::write(dir.join("src/lib.md"), b"").unwrap();
        std::fs::write(dir.join("NOTES.md"), b"").unwrap(); // stray root .md
        std::fs::write(dir.join("planning/status.md"), b"").unwrap(); // should be included

        let cfg = brain_only_config();
        let (corpus, diags) = crawl_corpus(&dir, &cfg);
        assert!(diags.is_empty(), "expected no diagnostics: {diags:?}");

        let stems: Vec<&str> = corpus.entries.iter().map(|e| e.stem.as_str()).collect();
        assert!(!stems.contains(&"lib"), "src/lib.md must not be in corpus");
        assert!(!stems.contains(&"NOTES"), "NOTES.md must not be in corpus");
        assert!(
            stems.contains(&"status"),
            "planning/status.md must be in corpus"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corpus_excludes_ephemeral_files() {
        let dir = std::env::temp_dir().join("mev-corpus-ephemeral");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("planning")).unwrap();
        std::fs::write(dir.join("planning/handoff.md"), b"").unwrap();
        std::fs::write(dir.join("planning/_draft.md"), b"").unwrap();
        std::fs::write(dir.join("planning/status.md"), b"").unwrap();

        let cfg = brain_only_config();
        let (corpus, diags) = crawl_corpus(&dir, &cfg);
        assert!(diags.is_empty(), "expected no diagnostics: {diags:?}");

        let stems: Vec<&str> = corpus.entries.iter().map(|e| e.stem.as_str()).collect();
        assert!(!stems.contains(&"handoff"), "handoff.md must be excluded");
        assert!(!stems.contains(&"_draft"), "_draft.md must be excluded");
        assert!(stems.contains(&"status"), "status.md must be included");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corpus_entries_carry_scope() {
        let dir = std::env::temp_dir().join("mev-corpus-scope");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("planning")).unwrap();
        std::fs::write(dir.join("planning/status.md"), b"").unwrap();
        std::fs::write(dir.join("README.md"), b"").unwrap();

        let cfg = brain_only_config();
        let (corpus, diags) = crawl_corpus(&dir, &cfg);
        assert!(diags.is_empty(), "expected no diagnostics: {diags:?}");

        for entry in &corpus.entries {
            assert_eq!(
                entry.scope, "brain",
                "all entries should have scope 'brain', got: {}",
                entry.scope
            );
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corpus_prunes_skip_dirs() {
        let dir = std::env::temp_dir().join("mev-corpus-skip-dirs");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("planning")).unwrap();
        std::fs::create_dir_all(dir.join("trees/some-worktree/planning")).unwrap();
        std::fs::write(dir.join("planning/status.md"), b"").unwrap();
        std::fs::write(dir.join("trees/some-worktree/planning/status.md"), b"").unwrap();

        let cfg = brain_only_config();
        let (corpus, diags) = crawl_corpus(&dir, &cfg);
        assert!(diags.is_empty(), "expected no diagnostics: {diags:?}");

        let rels: Vec<String> = corpus
            .entries
            .iter()
            .map(|e| e.rel.display().to_string())
            .collect();
        assert!(
            rels.iter().any(|r| r == "planning/status.md"),
            "expected planning/status.md in corpus"
        );
        assert!(
            !rels.iter().any(|r| r.contains("trees")),
            "trees/ subtree must be pruned by skip_dirs"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- has_nested_git ---

    #[test]
    fn has_nested_git_true_when_git_present() {
        let dir = std::env::temp_dir().join("mev-brain-nested-git-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Create a .git marker inside the directory.
        std::fs::create_dir_all(dir.join(".git")).unwrap();

        assert!(has_nested_git(&dir));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn has_nested_git_false_when_no_git() {
        let dir = std::env::temp_dir().join("mev-brain-no-git-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        assert!(!has_nested_git(&dir));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
