//! CLI-level test for `mev carryover --grep <PATTERN>` (`MV.ticket.carryover-grep`, task 3).
//!
//! The filter itself and the report-level count/dedup-suppression behavior are covered at
//! the library level in `src/brain/carryover.rs`'s unit tests and `tests/it/brain_carryover.rs`.
//! This file only asserts the CLI wiring: the flag parses on the real clap `Carryover`
//! variant and actually reaches the sweep handler — driving the built binary directly
//! (`CARGO_BIN_EXE_mev`), the same pattern `tests/it/validate_cli_flags.rs` uses.

use std::fs;
use std::path::Path;
use std::process::Command;

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_mev"))
        .args(args)
        .output()
        .expect("run mev")
}

fn temp_dir(suffix: &str) -> std::path::PathBuf {
    let dir = mev::testsupport::unique_temp_dir(&format!("mev-brain-carryover-grep-cli-{suffix}"));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_json(root: &Path, rel: &str, value: &serde_json::Value) {
    let full = root.join(rel);
    fs::create_dir_all(full.parent().unwrap()).unwrap();
    fs::write(
        &full,
        serde_json::to_string_pretty(value).unwrap().as_bytes(),
    )
    .unwrap();
}

/// A single-repo fixture with two `carryover[]` entries, one of which is uniquely
/// findable by a `--grep` pattern against its `slug`.
fn write_fixture(root: &Path) {
    let toml = r#"[vocab]
layer = ["brain", "engine", "factory", "console", "surface", "infra", "business", "content", "meta"]
status = ["active", "draft", "deprecated", "superseded", "archived"]

[crawl]
skip_dirs = ["target", "node_modules", ".git"]

[[repos]]
slug = "gamma"
tier = "primary"
repo_path = "repos/gamma"
status_file = "repos/gamma/planning/status.md"
cache_doc = "docs/projects/gamma.md"
heading = "Gamma"
"#;
    fs::write(root.join("brain.toml"), toml.as_bytes()).unwrap();

    let state = serde_json::json!({
        "repo": "gamma",
        "kind": "project",
        "updated": "2026-08-01",
        "focus": { "now": [], "next": [], "blocked": [] },
        "tracks": [],
        "carryover": [
            {
                "slug": "gamma-needle-entry",
                "scope": { "repo": "gamma" },
                "kind": "deferred",
                "text": "A uniquely findable entry for the --grep CLI test.",
                "clears_when": "a human reviews this manually and signs off",
                "created": "2026-06-01"
            },
            {
                "slug": "gamma-unrelated-entry",
                "scope": { "repo": "gamma" },
                "kind": "deferred",
                "text": "Nothing to do with the search pattern.",
                "clears_when": "a human reviews this manually and signs off",
                "created": "2026-06-01"
            }
        ]
    });
    write_json(root, "repos/gamma/planning/state.json", &state);
}

#[test]
fn grep_flag_parses_and_filters_to_the_matching_entry() {
    let dir = temp_dir("match");
    write_fixture(&dir);

    let out = run(&[
        "carryover",
        "--grep",
        "needle",
        "--json",
        dir.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "expected exit 0, got {:?}\nstderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    let report: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("stdout should be a single JSON report");

    let entries = report["entries"].as_array().expect("entries array");
    assert_eq!(
        entries.len(),
        1,
        "expected exactly the one matching entry, got: {report:#}"
    );
    assert_eq!(entries[0]["slug"], "gamma-needle-entry");
    assert_eq!(report["total"], 1, "total must describe the filtered set");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn grep_flag_matching_nothing_exits_zero_and_says_so() {
    let dir = temp_dir("no-match");
    write_fixture(&dir);

    let out = run(&[
        "carryover",
        "--grep",
        "no-such-pattern-anywhere",
        dir.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "a pattern matching nothing must still exit 0, got {:?}",
        out.status.code()
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.to_lowercase().contains("no-such-pattern-anywhere"),
        "expected the human summary to name the active pattern, got:\n{stdout}"
    );
    assert!(
        stdout.to_lowercase().contains("matched"),
        "expected the summary to distinguish 'matched nothing' from 'nothing to sweep', got:\n{stdout}"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn invalid_grep_regex_exits_non_zero_naming_the_pattern_and_the_error() {
    let dir = temp_dir("invalid-regex");
    write_fixture(&dir);

    let out = run(&["carryover", "--grep", "(unclosed", dir.to_str().unwrap()]);
    assert!(!out.status.success(), "an invalid regex must exit non-zero");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("(unclosed"),
        "expected the error to name the pattern, got:\n{stderr}"
    );
}
