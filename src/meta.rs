//! Struct- and frontmatter-level validation (Phase 1, Block C).
//!
//! `crawl.rs` walks the content tree and classifies each file; this module reads each
//! classified file's contents and validates its structure, keeping `crawl.rs` focused on
//! the walk. Block C lands incrementally:
//!
//! - **Task 1 (this commit):** establish the module, the per-file dispatch entry point
//!   ([`validate_file`]), and the read step ([`read_content`]) — surfacing IO/read failures
//!   as `error`-severity [`Diagnostic`]s rather than panicking, so one unreadable file never
//!   aborts the whole run.
//! - **Tasks 2-4:** add the per-kind serde structs (`ModuleMeta`, path `metadata.json`) and
//!   the field/enum/format checks, plus real-YAML MDX frontmatter parsing.
//! - **Task 5:** wire [`validate_file`] into `validate()` so the diagnostics reach the `Report`.

use crate::Diagnostic;
use crate::crawl::ContentFile;

/// Read a classified content file's contents from disk.
///
/// On any read failure (missing file, permission error, non-UTF-8 bytes) this returns a single
/// `error`-severity [`Diagnostic`] located at the file's relative path rather than panicking or
/// propagating an `Err` — so one unreadable file is reported and the run continues.
pub(crate) fn read_content(cf: &ContentFile) -> Result<String, Diagnostic> {
    std::fs::read_to_string(&cf.path)
        .map_err(|e| Diagnostic::error(cf.rel.clone(), "", format!("could not read file: {e}")))
}

/// Validate a single classified content file, dispatching on its [`FileKind`].
///
/// Returns every diagnostic the file produces (an empty vector means the file is clean).
///
/// Task 1 reads the file and surfaces read failures only; the per-kind struct, enum, format,
/// and frontmatter checks are layered in by Tasks 2-4 on top of the successfully-read contents.
pub fn validate_file(cf: &ContentFile) -> Vec<Diagnostic> {
    let _contents = match read_content(cf) {
        Ok(contents) => contents,
        Err(diag) => return vec![diag],
    };

    // Per-kind struct/frontmatter checks are added in Tasks 2-4.
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Severity;
    use crate::crawl::{FileKind, Locale};
    use std::path::PathBuf;

    fn temp_dir(suffix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mev-meta-{suffix}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn content_file(path: PathBuf, rel: &str, kind: FileKind) -> ContentFile {
        ContentFile {
            path,
            rel: PathBuf::from(rel),
            kind,
            path_id: "demo".to_string(),
            module_id: None,
            locale: Locale::En,
        }
    }

    #[test]
    fn read_content_ok_returns_file_body() {
        let dir = temp_dir("read-ok");
        let path = dir.join("metadata.json");
        std::fs::write(&path, b"{\"id\":\"demo\"}").unwrap();

        let cf = content_file(path, "paths/demo/metadata.json", FileKind::PathMetadataJson);
        let body = read_content(&cf).expect("expected a readable file");
        assert_eq!(body, "{\"id\":\"demo\"}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_content_missing_file_yields_error_diagnostic() {
        let cf = content_file(
            PathBuf::from("/nonexistent/mev/does-not-exist.json"),
            "paths/demo/metadata.json",
            FileKind::PathMetadataJson,
        );

        let diag = read_content(&cf).expect_err("expected a read failure");
        assert_eq!(diag.severity, Severity::Error);
        assert_eq!(diag.file, PathBuf::from("paths/demo/metadata.json"));
        assert_eq!(diag.locator, "");
        assert!(
            diag.message.starts_with("could not read file:"),
            "unexpected message: {}",
            diag.message
        );
    }

    #[test]
    fn validate_file_surfaces_read_failure_as_single_error() {
        let cf = content_file(
            PathBuf::from("/nonexistent/mev/missing.mdx"),
            "paths/demo/modules/01-intro.mdx",
            FileKind::ModuleMdx,
        );

        let diags = validate_file(&cf);
        assert_eq!(
            diags.len(),
            1,
            "expected exactly one read-failure diagnostic"
        );
        assert_eq!(diags[0].severity, Severity::Error);
        assert_eq!(
            diags[0].file,
            PathBuf::from("paths/demo/modules/01-intro.mdx")
        );
    }

    #[test]
    fn validate_file_readable_file_is_clean_in_task1() {
        // Task 1 only reads; per-kind checks arrive in Tasks 2-4, so a readable file is clean.
        let dir = temp_dir("readable-clean");
        let path = dir.join("01-intro.mdx");
        std::fs::write(&path, b"---\ntitle: x\n---\nbody").unwrap();

        let cf = content_file(path, "paths/demo/modules/01-intro.mdx", FileKind::ModuleMdx);
        assert!(validate_file(&cf).is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
