//! Integration tests for `mev validate`'s `--blog` / `--lint` CLI wiring (`MV.12.A` task 4).
//!
//! The library entry points (`mev::validate_blog`, `mev::validate_with_lint`) are covered by
//! `tests/blog_validate.rs`. This file covers the layer above them — flag parsing, which
//! validator each flag dispatches to, the per-flag positional-path default, the `--json`
//! envelope's `validator` label, and exit codes — by driving the built binary directly
//! (`CARGO_BIN_EXE_mev`), the same pattern `tests/doc_cli.rs` uses.
//!
//! Added by `/close-out`'s coverage gap scan: the CLI dispatch was verified by hand during
//! integration but had no test, and the conditional path default in particular is the kind of
//! wiring that silently regresses.

use std::fs;
use std::path::Path;
use std::process::Command;

/// Build a minimal blog tree: one clean EN post with a pt-BR counterpart, so a `--blog` run
/// over it is clean (no parity warning, no frontmatter error).
fn setup_blog(root: &Path) {
    let published = root.join("blog/published");
    fs::create_dir_all(published.join("pt-BR")).unwrap();
    let post = "---\ntitle: \"T\"\ndate: \"2026-01-01\"\nexcerpt: \"e\"\n---\n\nBody text.\n";
    fs::write(published.join("clean.mdx"), post).unwrap();
    fs::write(published.join("pt-BR/clean.mdx"), post).unwrap();
}

/// A post whose only defect is an untagged fence — a warning, never an error, so a run over
/// this tree must still exit 0.
fn setup_blog_with_warning(root: &Path) {
    let published = root.join("blog/published");
    fs::create_dir_all(published.join("pt-BR")).unwrap();
    let post =
        "---\ntitle: \"T\"\ndate: \"2026-01-01\"\nexcerpt: \"e\"\n---\n\n```\nuntagged\n```\n";
    fs::write(published.join("clean.mdx"), post).unwrap();
    fs::write(published.join("pt-BR/clean.mdx"), post).unwrap();
}

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_mev"))
        .args(args)
        .output()
        .expect("run mev")
}

#[test]
fn blog_flag_validates_the_blog_tree_and_exits_zero_when_clean() {
    let tmp = tempfile::tempdir().unwrap();
    setup_blog(tmp.path());
    let published = tmp.path().join("blog/published");

    let out = run(&["validate", "--blog", published.to_str().unwrap()]);
    let stdout = String::from_utf8_lossy(&out.stdout);

    assert!(
        out.status.success(),
        "clean blog tree must exit 0, got {:?}\n{stdout}",
        out.status.code()
    );
    assert!(
        stdout.contains("0 error(s)"),
        "expected zero errors, got: {stdout}"
    );
}

#[test]
fn blog_flag_reports_blog_codes_not_learn_module_codes() {
    let tmp = tempfile::tempdir().unwrap();
    let published = tmp.path().join("blog/published");
    fs::create_dir_all(&published).unwrap();
    // Missing every required frontmatter field.
    fs::write(published.join("bad.mdx"), "no frontmatter here\n").unwrap();

    let out = run(&["validate", "--blog", published.to_str().unwrap()]);
    let stdout = String::from_utf8_lossy(&out.stdout);

    assert!(
        stdout.contains("E_BLOG_MALFORMED_FRONTMATTER") || stdout.contains("E_BLOG_MISSING_FIELD"),
        "expected a blog frontmatter code, got: {stdout}"
    );
    assert_eq!(
        out.status.code(),
        Some(1),
        "an error-severity diagnostic must exit 1"
    );
}

#[test]
fn blog_warnings_alone_do_not_change_the_exit_code() {
    let tmp = tempfile::tempdir().unwrap();
    setup_blog_with_warning(tmp.path());
    let published = tmp.path().join("blog/published");

    let out = run(&["validate", "--blog", published.to_str().unwrap()]);
    let stdout = String::from_utf8_lossy(&out.stdout);

    assert!(
        stdout.contains("W_LINT_UNTAGGED_CODE_BLOCK"),
        "expected the untagged-fence warning, got: {stdout}"
    );
    assert!(
        out.status.success(),
        "warnings alone must still exit 0, got {:?}",
        out.status.code()
    );
}

#[test]
fn json_envelope_carries_the_blog_validator_label() {
    let tmp = tempfile::tempdir().unwrap();
    setup_blog(tmp.path());
    let published = tmp.path().join("blog/published");

    let out = run(&["--json", "validate", "--blog", published.to_str().unwrap()]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout must be JSON: {e}\n{stdout}"));

    assert_eq!(
        parsed["validator"], "blog",
        "the --blog path must set validator= \"blog\", got: {stdout}"
    );
}

#[test]
fn json_envelope_keeps_the_learn_ai_label_without_the_blog_flag() {
    let tmp = tempfile::tempdir().unwrap();
    let learn = tmp.path().join("learn");
    fs::create_dir_all(&learn).unwrap();

    let out = run(&["--json", "validate", learn.to_str().unwrap()]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout must be JSON: {e}\n{stdout}"));

    assert_eq!(
        parsed["validator"], "learn-ai",
        "the default path must keep validator=learn-ai, got: {stdout}"
    );
}

#[test]
fn lint_flag_adds_lint_codes_over_learn_modules_and_bare_validate_does_not() {
    let tmp = tempfile::tempdir().unwrap();
    let modules = tmp.path().join("paths/demo/modules");
    fs::create_dir_all(&modules).unwrap();
    fs::write(
        tmp.path().join("paths/demo/metadata.json"),
        "{\"title\":\"Demo\"}\n",
    )
    .unwrap();
    // A module with an untagged fence — only the lint pass should notice it.
    fs::write(
        modules.join("01-intro.mdx"),
        "# Intro\n\n```\nuntagged\n```\n",
    )
    .unwrap();
    fs::write(modules.join("01-intro.json"), "{}\n").unwrap();

    let root = tmp.path().to_str().unwrap();
    let with_lint = run(&["validate", "--lint", root]);
    let without = run(&["validate", root]);

    let lint_out = String::from_utf8_lossy(&with_lint.stdout);
    let bare_out = String::from_utf8_lossy(&without.stdout);

    assert!(
        lint_out.contains("W_LINT_UNTAGGED_CODE_BLOCK"),
        "--lint must report the untagged fence over modules, got: {lint_out}"
    );
    assert!(
        !bare_out.contains("W_LINT_UNTAGGED_CODE_BLOCK"),
        "bare `validate` must NOT report lint codes — this is the byte-identical guarantee: {bare_out}"
    );
}

#[test]
fn help_lists_both_new_flags() {
    let out = run(&["validate", "--help"]);
    let stdout = String::from_utf8_lossy(&out.stdout);

    assert!(
        stdout.contains("--blog"),
        "--help must list --blog: {stdout}"
    );
    assert!(
        stdout.contains("--lint"),
        "--help must list --lint: {stdout}"
    );
}
