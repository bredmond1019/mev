//! Structural coverage check (Phase 3, Block L): bidirectional `index.md` ↔
//! directory consistency (D17 / CLAUDE.md Standing Rule 7).
//!
//! Every corpus file in a directory must appear in that directory's `index.md`
//! (orphan detection), and every `index.md` row must point at a file that exists
//! on disk (dangling-row detection). Both directions are per-directory, direct
//! children only — subdirectories are covered by their own `index.md`.
//!
//! # Diagnostic locator codes
//! - `E_STRUCT_ORPHAN_FILE` — a corpus file present in a directory but not
//!   referenced by that directory's `index.md`. Located at the orphan file.
//! - `E_STRUCT_DANGLING_ROW` — an `index.md` row (markdown or `file://` link)
//!   whose target does not exist on disk. Located at the `index.md`.
//!
//! Directories with no `index.md` corpus member are skipped entirely — no
//! coverage obligation, so no orphan flags. `[[wikilink]]`, external
//! (`http(s)://` etc.), and out-of-corpus-root link targets are ignored (owned
//! elsewhere / out of scope for this check).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::Diagnostic;
use crate::brain::crawl::{Corpus, CorpusEntry};
use crate::brain::links::{self, LinkKind};

// ---------------------------------------------------------------------------
// Path normalization helper
// ---------------------------------------------------------------------------

/// Lexically normalise a path by resolving `.` and `..` components without
/// touching the filesystem (no `canonicalize` — the target may not exist).
///
/// Ensures `./foo.md`, `foo.md`, and mixed-separator variants of the same
/// target compare equal via [`PathBuf`] component comparison rather than raw
/// string equality.
fn normalize(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {} // skip `.`
            Component::ParentDir => {
                if !out.pop() {
                    out.push(Component::ParentDir);
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// Return `true` if `entry` is an `index.md` corpus member.
fn is_index_md(entry: &CorpusEntry) -> bool {
    entry.path.file_name().and_then(|n| n.to_str()) == Some("index.md")
}

// ---------------------------------------------------------------------------
// check_structure
// ---------------------------------------------------------------------------

/// Run the bidirectional `index.md` ↔ directory structural coverage check over
/// `corpus`, emitting `E_STRUCT_ORPHAN_FILE` / `E_STRUCT_DANGLING_ROW` diagnostics.
///
/// `root` bounds the "in corpus" test for dangling-row detection: a resolved
/// link target outside `root` is ignored (not this check's job).
pub fn check_structure(corpus: &Corpus, root: &Path) -> Vec<Diagnostic> {
    let mut diags: Vec<Diagnostic> = Vec::new();
    let root_norm = normalize(root);

    // dir (normalized, absolute) -> its index.md CorpusEntry.
    let mut index_by_dir: HashMap<PathBuf, &CorpusEntry> = HashMap::new();
    // dir (normalized, absolute) -> direct-child corpus entries (excluding index.md).
    let mut children_by_dir: HashMap<PathBuf, Vec<&CorpusEntry>> = HashMap::new();

    for entry in &corpus.entries {
        let Some(dir) = entry.path.parent() else {
            continue;
        };
        let dir_norm = normalize(dir);
        if is_index_md(entry) {
            index_by_dir.insert(dir_norm, entry);
        } else {
            children_by_dir.entry(dir_norm).or_default().push(entry);
        }
    }

    for (dir, index_entry) in &index_by_dir {
        let contents = match std::fs::read_to_string(&index_entry.path) {
            Ok(c) => c,
            // Graceful degrade — unreadable index.md is reported elsewhere (OKF pass).
            Err(_) => continue,
        };

        let extracted = links::extract_links(&contents);
        let mut covered: HashSet<PathBuf> = HashSet::new();

        for link in &extracted {
            let resolved = match link.kind {
                LinkKind::Markdown => normalize(&dir.join(&link.target)),
                LinkKind::FileUri => {
                    let path_str = link.target.strip_prefix("file://").unwrap_or(&link.target);
                    normalize(Path::new(path_str))
                }
                LinkKind::WikiLink => continue, // out of scope (owned by MV.3.K)
            };

            if !resolved.starts_with(&root_norm) {
                continue; // outside corpus root — not this check's job
            }

            covered.insert(resolved.clone());

            if !resolved.exists() {
                diags.push(Diagnostic::error(
                    &index_entry.rel,
                    "E_STRUCT_DANGLING_ROW",
                    format!(
                        "index.md row points at a nonexistent file: '{}' (resolved: '{}')",
                        link.raw,
                        resolved.display()
                    ),
                ));
            }
        }

        if let Some(children) = children_by_dir.get(dir) {
            for child in children {
                let child_norm = normalize(&child.path);
                if !covered.contains(&child_norm) {
                    diags.push(Diagnostic::error(
                        &child.rel,
                        "E_STRUCT_ORPHAN_FILE",
                        format!(
                            "file '{}' is not referenced by '{}'",
                            child.rel.display(),
                            index_entry.rel.display()
                        ),
                    ));
                }
            }
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
    use crate::brain::okf::OkfFrontmatter;
    use crate::shared::extract_frontmatter;

    /// Write `content` to a temp file and return a [`CorpusEntry`] for it.
    fn write_corpus_entry(dir: &Path, rel_str: &str, content: &str) -> CorpusEntry {
        let full = dir.join(rel_str);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&full, content.as_bytes()).unwrap();
        let metadata = extract_frontmatter(content)
            .and_then(|yaml| serde_yaml::from_str::<OkfFrontmatter>(yaml).ok());
        CorpusEntry {
            path: full,
            rel: PathBuf::from(rel_str),
            stem: Path::new(rel_str)
                .file_stem()
                .unwrap()
                .to_str()
                .unwrap()
                .to_string(),
            scope: "brain".to_string(),
            metadata,
        }
    }

    fn tmp_dir(name: &str) -> PathBuf {
        let dir = crate::testsupport::unique_temp_dir(name);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    // --- clean dir: index.md covers its one sibling file -> no diagnostics ---

    #[test]
    fn clean_dir_produces_no_diagnostics() {
        let dir = tmp_dir("mev-structure-clean");

        let index = write_corpus_entry(&dir, "docs/index.md", "See [status](status.md).");
        let status = write_corpus_entry(&dir, "docs/status.md", "Body.");

        let corpus = Corpus {
            entries: vec![index, status],
        };
        let diags = check_structure(&corpus, &dir);
        assert!(diags.is_empty(), "expected no diagnostics, got: {diags:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- one orphan file: not referenced by index.md -> E_STRUCT_ORPHAN_FILE ---

    #[test]
    fn orphan_file_is_flagged() {
        let dir = tmp_dir("mev-structure-orphan");

        let index = write_corpus_entry(&dir, "docs/index.md", "Nothing here yet.");
        let orphan = write_corpus_entry(&dir, "docs/orphan.md", "Body.");

        let corpus = Corpus {
            entries: vec![index, orphan],
        };
        let diags = check_structure(&corpus, &dir);

        assert_eq!(diags.len(), 1, "expected 1 diagnostic, got: {diags:?}");
        assert_eq!(diags[0].locator, "E_STRUCT_ORPHAN_FILE");
        assert_eq!(diags[0].file, PathBuf::from("docs/orphan.md"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- one dangling row: index.md points at a file that does not exist ---

    #[test]
    fn dangling_row_is_flagged() {
        let dir = tmp_dir("mev-structure-dangling");

        let index = write_corpus_entry(&dir, "docs/index.md", "See [gone](gone.md).");

        let corpus = Corpus {
            entries: vec![index],
        };
        let diags = check_structure(&corpus, &dir);

        assert_eq!(diags.len(), 1, "expected 1 diagnostic, got: {diags:?}");
        assert_eq!(diags[0].locator, "E_STRUCT_DANGLING_ROW");
        assert_eq!(diags[0].file, PathBuf::from("docs/index.md"));
        assert!(diags[0].message.contains("gone.md"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- both an orphan file and a dangling row in the same directory ---

    #[test]
    fn orphan_and_dangling_row_both_flagged() {
        let dir = tmp_dir("mev-structure-both");

        let index = write_corpus_entry(&dir, "docs/index.md", "See [gone](gone.md).");
        let orphan = write_corpus_entry(&dir, "docs/orphan.md", "Body.");

        let corpus = Corpus {
            entries: vec![index, orphan],
        };
        let mut diags = check_structure(&corpus, &dir);
        diags.sort_by(|a, b| a.locator.cmp(&b.locator));

        assert_eq!(diags.len(), 2, "expected 2 diagnostics, got: {diags:?}");
        assert_eq!(diags[0].locator, "E_STRUCT_DANGLING_ROW");
        assert_eq!(diags[1].locator, "E_STRUCT_ORPHAN_FILE");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- directory with no index.md -> no orphan flags ---

    #[test]
    fn directory_without_index_md_produces_no_diagnostics() {
        let dir = tmp_dir("mev-structure-no-index");

        let lonely = write_corpus_entry(&dir, "docs/lonely.md", "Body.");

        let corpus = Corpus {
            entries: vec![lonely],
        };
        let diags = check_structure(&corpus, &dir);
        assert!(diags.is_empty(), "expected no diagnostics, got: {diags:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- './'-prefixed / mixed-separator link normalizes to the same target ---

    #[test]
    fn dot_prefixed_link_normalizes_and_covers_child() {
        let dir = tmp_dir("mev-structure-dot-prefix");

        let index = write_corpus_entry(&dir, "docs/index.md", "See [status](./status.md).");
        let status = write_corpus_entry(&dir, "docs/status.md", "Body.");

        let corpus = Corpus {
            entries: vec![index, status],
        };
        let diags = check_structure(&corpus, &dir);
        assert!(
            diags.is_empty(),
            "expected './'-prefixed link to cover the file, got: {diags:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- wikilink / external targets never produce E_STRUCT_* diagnostics ---

    #[test]
    fn wikilink_and_external_targets_ignored() {
        let dir = tmp_dir("mev-structure-wikilink-external");

        let index = write_corpus_entry(
            &dir,
            "docs/index.md",
            "See [[some-slug]] and [ext](https://example.com).",
        );
        let orphan = write_corpus_entry(&dir, "docs/orphan.md", "Body.");

        let corpus = Corpus {
            entries: vec![index, orphan],
        };
        let diags = check_structure(&corpus, &dir);

        // orphan.md is still uncovered (wikilink/external don't count), so exactly
        // one E_STRUCT_ORPHAN_FILE — never an E_STRUCT_DANGLING_ROW from the
        // wikilink/external targets themselves.
        assert_eq!(diags.len(), 1, "expected 1 diagnostic, got: {diags:?}");
        assert_eq!(diags[0].locator, "E_STRUCT_ORPHAN_FILE");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
