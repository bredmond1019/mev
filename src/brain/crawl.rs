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

/// Return `true` if a directory name appears in the caller-supplied `skip_dirs` slice.
///
/// The slice is sourced from `brain.toml`'s `[crawl].skip_dirs` — no names are hardcoded here.
pub(crate) fn is_blocklisted_name(name: &str, skip_dirs: &[String]) -> bool {
    skip_dirs.iter().any(|d| d == name)
}

/// Return `true` if a file name is on the file blocklist.
///
/// Blocklisted files: tool config files (`CLAUDE.md`, `CLAUDE.local.md`, `GEMINI.md`) and
/// transient session artifacts (`handoff.md`) that are never OKF docs.
pub(crate) fn is_blocklisted_file(name: &str) -> bool {
    matches!(
        name,
        "CLAUDE.md" | "CLAUDE.local.md" | "GEMINI.md" | "handoff.md"
    )
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
            if is_blocklisted_name(&name, skip_dirs) {
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
        assert!(is_blocklisted_name("target", &skip));
        assert!(is_blocklisted_name("node_modules", &skip));
        assert!(is_blocklisted_name(".git", &skip));
        assert!(is_blocklisted_name(".claude", &skip));
        assert!(is_blocklisted_name(".repo-backups", &skip));
        assert!(is_blocklisted_name(".agent", &skip));
    }

    #[test]
    fn ordinary_names_are_allowed() {
        let skip = standard_skip_dirs();
        assert!(!is_blocklisted_name("docs", &skip));
        assert!(!is_blocklisted_name("src", &skip));
        assert!(!is_blocklisted_name("planning", &skip));
        assert!(!is_blocklisted_name("README.md", &skip));
        assert!(!is_blocklisted_name("target-extra", &skip)); // prefix match must not fire
    }

    #[test]
    fn empty_skip_dirs_allows_all_names() {
        let skip: Vec<String> = vec![];
        assert!(!is_blocklisted_name("target", &skip));
        assert!(!is_blocklisted_name(".git", &skip));
        assert!(!is_blocklisted_name("node_modules", &skip));
    }

    #[test]
    fn custom_skip_dirs_are_respected() {
        let skip = vec!["custom-dir".to_string(), "another".to_string()];
        assert!(is_blocklisted_name("custom-dir", &skip));
        assert!(is_blocklisted_name("another", &skip));
        assert!(!is_blocklisted_name("target", &skip));
    }

    // --- is_blocklisted_file ---

    #[test]
    fn blocklisted_files_are_rejected() {
        assert!(is_blocklisted_file("CLAUDE.md"));
        assert!(is_blocklisted_file("CLAUDE.local.md"));
        assert!(is_blocklisted_file("GEMINI.md"));
        assert!(is_blocklisted_file("handoff.md"));
    }

    #[test]
    fn ordinary_files_are_allowed() {
        assert!(!is_blocklisted_file("status.md"));
        assert!(!is_blocklisted_file("README.md"));
        assert!(!is_blocklisted_file("context.md"));
        assert!(!is_blocklisted_file("claude-notes.md")); // prefix match must not fire
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
