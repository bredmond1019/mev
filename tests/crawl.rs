//! Fixture-driven integration tests for the content-tree crawl and classification logic.
//!
//! Each test builds a small temp-dir tree, calls [`mev::crawl`], and asserts on the resulting
//! corpus and diagnostics. These tests exercise `crawl()` end-to-end against real files —
//! the unit tests in `src/crawl.rs` cover helper functions in isolation.

use std::fs;
use std::path::Path;

use mev::{FileKind, Locale, Severity, crawl};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create a file and all parent directories.
fn touch(root: &Path, rel: &str) {
    let full = root.join(rel);
    fs::create_dir_all(full.parent().unwrap()).unwrap();
    fs::write(&full, b"").unwrap();
}

/// Make a fresh temp dir with a unique name for this test.
fn temp_dir(suffix: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("mev-crawl-{suffix}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

// ---------------------------------------------------------------------------
// 1. Good modules — correct classification and grouping
// ---------------------------------------------------------------------------

#[test]
fn good_modules_classified_correctly() {
    let dir = temp_dir("good");

    touch(&dir, "paths/intro/metadata.json");
    touch(&dir, "paths/intro/modules/01-intro.json");
    touch(&dir, "paths/intro/modules/01-intro.mdx");

    let (corpus, diags) = crawl(&dir);

    // No filename violations.
    assert_eq!(diags.len(), 0, "expected no diagnostics, got: {diags:#?}");

    // Corpus contains exactly 3 files.
    assert_eq!(corpus.files.len(), 3, "expected 3 content files in corpus");

    // path_ids() returns exactly ["intro"].
    assert_eq!(corpus.path_ids(), vec!["intro"]);

    // metadata.json is PathMetadataJson with no module_id.
    let meta = corpus
        .files
        .iter()
        .find(|f| f.kind == FileKind::PathMetadataJson)
        .expect("PathMetadataJson not found");
    assert_eq!(meta.path_id, "intro");
    assert!(meta.module_id.is_none());
    assert_eq!(meta.locale, Locale::En);

    // 01-intro.json is LearnModuleJson with module_id "01-intro".
    let json = corpus
        .get("intro", "01-intro", Locale::En, FileKind::LearnModuleJson)
        .expect("LearnModuleJson not found");
    assert_eq!(json.path_id, "intro");
    assert_eq!(json.module_id.as_deref(), Some("01-intro"));

    // 01-intro.mdx is ModuleMdx with module_id "01-intro".
    let mdx = corpus
        .get("intro", "01-intro", Locale::En, FileKind::ModuleMdx)
        .expect("ModuleMdx not found");
    assert_eq!(mdx.path_id, "intro");
    assert_eq!(mdx.module_id.as_deref(), Some("01-intro"));

    // modules_for returns json + mdx (2 module files), excludes metadata.
    let modules = corpus.modules_for("intro", Locale::En);
    assert_eq!(modules.len(), 2, "expected 2 module files for 'intro'");

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 2. Bad filenames — one Error diagnostic per violation
// ---------------------------------------------------------------------------

/// A filename with a space must produce exactly one Severity::Error.
#[test]
fn bad_filename_spaces_emits_error() {
    let dir = temp_dir("bad-spaces");

    // Place a module file whose name contains a space.
    touch(&dir, "paths/intro/modules/01-bad name.json");

    let (_corpus, diags) = crawl(&dir);

    let errors: Vec<_> = diags
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .collect();

    assert!(
        !errors.is_empty(),
        "expected at least one Error for filename with spaces"
    );
    assert!(
        errors.iter().any(|d| d.message.contains("spaces")),
        "expected 'spaces' in diagnostic message, got: {errors:#?}"
    );
}

/// An uppercase filename must produce exactly one Severity::Error.
#[test]
fn bad_filename_uppercase_emits_error() {
    let dir = temp_dir("bad-upper");

    // Place a module file whose name contains an uppercase character.
    touch(&dir, "paths/intro/modules/01-Intro.json");

    let (_corpus, diags) = crawl(&dir);

    let errors: Vec<_> = diags
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .collect();

    // We expect at least one error for the uppercase filename.
    assert!(
        !errors.is_empty(),
        "expected at least one Error for uppercase filename"
    );
    assert!(
        errors.iter().any(|d| d.message.contains("lowercase")),
        "expected 'lowercase' in diagnostic message, got: {errors:#?}"
    );

    let _ = fs::remove_dir_all(&dir);
}

/// A module filename missing the `NN-` prefix must produce exactly one Severity::Error.
#[test]
fn bad_filename_missing_nn_prefix_emits_error() {
    let dir = temp_dir("bad-prefix");

    // Place a module file without the two-digit prefix.
    touch(&dir, "paths/intro/modules/intro.json");

    let (_corpus, diags) = crawl(&dir);

    let errors: Vec<_> = diags
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .collect();

    assert!(
        !errors.is_empty(),
        "expected at least one Error for missing NN- prefix"
    );
    assert!(
        errors.iter().any(|d| d.message.contains("NN-slug")),
        "expected 'NN-slug' in diagnostic message, got: {errors:#?}"
    );

    let _ = fs::remove_dir_all(&dir);
}

/// Bad-filename files are still added to the corpus (not dropped).
#[test]
fn bad_filename_file_still_in_corpus() {
    let dir = temp_dir("bad-in-corpus");

    touch(&dir, "paths/intro/modules/intro.json"); // missing NN- prefix

    let (corpus, diags) = crawl(&dir);

    // The file must appear in the corpus.
    assert_eq!(corpus.files.len(), 1, "file should still be in corpus");
    // And there should be an error diagnostic.
    assert!(
        diags.iter().any(|d| d.severity == Severity::Error),
        "expected Error diagnostic for bad filename"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 3. Non-content files are skipped — zero diagnostics, not in corpus
// ---------------------------------------------------------------------------

#[test]
fn non_content_files_are_skipped() {
    let dir = temp_dir("non-content");

    // README.md at the root — not under paths/
    touch(&dir, "README.md");
    // schemas/ directory — not under paths/
    touch(&dir, "schemas/learn-module.json");
    // shared/ directory — not under paths/
    touch(&dir, "shared/helpers.ts");
    // dotfile at root
    touch(&dir, ".gitignore");

    let (corpus, diags) = crawl(&dir);

    assert_eq!(
        corpus.files.len(),
        0,
        "non-content files must not appear in corpus"
    );
    assert_eq!(
        diags.len(),
        0,
        "non-content files must not produce any diagnostics"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 4. Empty tree — clean report
// ---------------------------------------------------------------------------

#[test]
fn empty_tree_produces_empty_corpus_and_no_diagnostics() {
    let dir = temp_dir("empty");

    let (corpus, diags) = crawl(&dir);

    assert_eq!(
        corpus.files.len(),
        0,
        "empty tree must produce an empty corpus"
    );
    assert_eq!(diags.len(), 0, "empty tree must produce no diagnostics");

    let _ = fs::remove_dir_all(&dir);
}
