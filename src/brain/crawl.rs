//! Brain-repo Markdown crawl.
//!
//! Walks a directory tree collecting every `*.md` file, applying a two-layer skip-list:
//!
//! **Name blocklist** — any directory named `target`, `node_modules`, or `.git` is pruned
//! (its entire subtree is skipped).
//!
//! **Nested-git rule** — any directory at `depth() > 0` that contains its own `.git` entry
//! is pruned.  The `depth() > 0` guard exempts the brain root itself, which is a git repo
//! but must not be pruned.
//!
//! [`crawl_brain`] returns `(Vec<MdFile>, Vec<Diagnostic>)` mirroring the learn-ai crawl
//! shape.  Walk/IO errors are surfaced as [`Diagnostic`]s rather than propagated as `Err`.

use std::path::{Path, PathBuf};

use crate::Diagnostic;

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
// Pruning helpers
// ---------------------------------------------------------------------------

/// Return `true` if a directory name is on the name blocklist.
///
/// Blocklisted names: `target`, `node_modules`, `.git`.
pub(crate) fn is_blocklisted_name(name: &str) -> bool {
    matches!(name, "target" | "node_modules" | ".git")
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
/// Pruning rules (applied at the directory level so entire subtrees are skipped):
/// - Any directory named `target`, `node_modules`, or `.git`.
/// - Any directory at depth > 0 that contains its own `.git` entry.
pub fn crawl_brain(root: &Path) -> (Vec<MdFile>, Vec<Diagnostic>) {
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
            if is_blocklisted_name(&name) {
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
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- is_blocklisted_name ---

    #[test]
    fn blocklisted_names_are_rejected() {
        assert!(is_blocklisted_name("target"));
        assert!(is_blocklisted_name("node_modules"));
        assert!(is_blocklisted_name(".git"));
    }

    #[test]
    fn ordinary_names_are_allowed() {
        assert!(!is_blocklisted_name("docs"));
        assert!(!is_blocklisted_name("src"));
        assert!(!is_blocklisted_name("planning"));
        assert!(!is_blocklisted_name("README.md"));
        assert!(!is_blocklisted_name("target-extra")); // prefix match must not fire
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
