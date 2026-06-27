//! Integration tests for the brain-repo crawl.
//!
//! Each test builds a small temp-dir fixture, calls [`mev::crawl_brain`], and asserts on the
//! returned `Vec<MdFile>` and diagnostics.  The unit tests in `src/brain/crawl.rs` cover the
//! pruning helpers in isolation; these tests exercise the end-to-end walk behaviour.

use std::fs;
use std::path::Path;

use mev::crawl_brain;

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
    let dir = std::env::temp_dir().join(format!("mev-brain-crawl-{suffix}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

// ---------------------------------------------------------------------------
// (a) A root-level .md file is found
// ---------------------------------------------------------------------------

#[test]
fn root_level_md_is_found() {
    let dir = temp_dir("root-md");

    touch(&dir, "README.md");

    let (files, diags) = crawl_brain(&dir);

    assert_eq!(diags.len(), 0, "expected no diagnostics, got: {diags:#?}");
    assert_eq!(files.len(), 1, "expected 1 MdFile, got {files:#?}");
    assert_eq!(files[0].stem, "README");
    assert_eq!(files[0].rel.to_str().unwrap(), "README.md");

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// (b) A .md inside a target/ dir is pruned
// ---------------------------------------------------------------------------

#[test]
fn md_inside_target_is_pruned() {
    let dir = temp_dir("target-prune");

    touch(&dir, "docs/notes.md");
    touch(&dir, "target/release/something.md");

    let (files, diags) = crawl_brain(&dir);

    assert_eq!(diags.len(), 0, "expected no diagnostics, got: {diags:#?}");
    assert_eq!(files.len(), 1, "expected only notes.md to be found");
    assert_eq!(files[0].stem, "notes");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn md_inside_node_modules_is_pruned() {
    let dir = temp_dir("node-modules-prune");

    touch(&dir, "index.md");
    touch(&dir, "node_modules/pkg/README.md");

    let (files, diags) = crawl_brain(&dir);

    assert_eq!(diags.len(), 0, "expected no diagnostics");
    assert_eq!(files.len(), 1, "expected only index.md to be found");
    assert_eq!(files[0].stem, "index");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn md_inside_dot_git_is_pruned() {
    let dir = temp_dir("dot-git-prune");

    touch(&dir, "plan.md");
    touch(&dir, ".git/COMMIT_EDITMSG.md");

    let (files, diags) = crawl_brain(&dir);

    assert_eq!(diags.len(), 0, "expected no diagnostics");
    assert_eq!(files.len(), 1, "expected only plan.md to be found");
    assert_eq!(files[0].stem, "plan");

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// (c) A .md inside a nested-git sub-dir is pruned; root .md is still found
// ---------------------------------------------------------------------------

#[test]
fn md_in_nested_git_subdir_is_pruned_root_md_found() {
    let dir = temp_dir("nested-git");

    // Root-level markdown — must be found.
    touch(&dir, "overview.md");

    // A sub-project with its own .git — must be pruned entirely.
    let sub = dir.join("sub-project");
    fs::create_dir_all(&sub).unwrap();
    // The .git marker that makes this a nested repo.
    fs::create_dir_all(sub.join(".git")).unwrap();
    // A .md inside the sub-project — must be pruned.
    fs::write(sub.join("README.md"), b"").unwrap();

    let (files, diags) = crawl_brain(&dir);

    assert_eq!(diags.len(), 0, "expected no diagnostics, got: {diags:#?}");
    assert_eq!(
        files.len(),
        1,
        "expected only overview.md; nested-git sub-dir must be pruned. Got: {files:#?}"
    );
    assert_eq!(files[0].stem, "overview");

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// (d) Non-.md files are skipped
// ---------------------------------------------------------------------------

#[test]
fn non_md_files_are_skipped() {
    let dir = temp_dir("non-md");

    touch(&dir, "notes.txt");
    touch(&dir, "data.json");
    touch(&dir, "script.rs");
    touch(&dir, "readme.md"); // only this should be found

    let (files, diags) = crawl_brain(&dir);

    assert_eq!(diags.len(), 0, "expected no diagnostics");
    assert_eq!(files.len(), 1, "only .md files should be collected");
    assert_eq!(files[0].stem, "readme");

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// (e) MdFile.rel and MdFile.stem carry expected values
// ---------------------------------------------------------------------------

#[test]
fn md_file_rel_and_stem_are_correct() {
    let dir = temp_dir("rel-stem");

    touch(&dir, "docs/planning/status.md");

    let (files, diags) = crawl_brain(&dir);

    assert_eq!(diags.len(), 0, "expected no diagnostics");
    assert_eq!(files.len(), 1, "expected one MdFile");

    let f = &files[0];
    assert_eq!(f.stem, "status", "stem should be 'status'");
    assert_eq!(
        f.rel.to_str().unwrap(),
        "docs/planning/status.md",
        "rel should be relative to the crawl root"
    );
    assert!(
        f.path.is_absolute(),
        "path should be absolute, got: {}",
        f.path.display()
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Empty tree
// ---------------------------------------------------------------------------

#[test]
fn empty_tree_returns_no_files_and_no_diagnostics() {
    let dir = temp_dir("empty");

    let (files, diags) = crawl_brain(&dir);

    assert_eq!(files.len(), 0, "expected empty file list");
    assert_eq!(diags.len(), 0, "expected no diagnostics");

    let _ = fs::remove_dir_all(&dir);
}
