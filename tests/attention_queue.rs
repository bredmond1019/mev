//! Integration tests for `mev attention-queue`
//! (`MV.ticket.attention-queue-delivery` task 5).
//!
//! Drives the built binary directly (`CARGO_BIN_EXE_mev`), the same pattern
//! `tests/doc_cli.rs` / `tests/emit_state_lock.rs` use, since this command's
//! contract is the CLI surface: stdout is a JSON array, `--out` writes a
//! file, and the corpus load/effective-priority derivation it must reuse
//! (rather than re-derive) is only observable end-to-end.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn temp_dir(tag: &str) -> PathBuf {
    let d = mev::testsupport::unique_temp_dir(&format!("mev-attention-queue-cli-{tag}"));
    fs::create_dir_all(&d).unwrap();
    d
}

fn write_file(root: &Path, rel: &str, content: &str) {
    let target = root.join(rel);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&target, content.as_bytes()).unwrap();
}

fn write_json(root: &Path, rel: &str, value: &serde_json::Value) {
    write_file(root, rel, &serde_json::to_string_pretty(value).unwrap());
}

/// Minimal `brain.toml` registering a single leaf repo ("alpha") plus the
/// `[attention]` table left at its defaults, mirroring
/// `tests/emit_state_lock.rs::write_brain_toml`.
fn write_brain_toml(root: &Path) {
    let toml = r#"[vocab]
layer = ["brain", "engine", "factory", "console", "surface", "infra", "business", "content", "meta"]
status = ["active", "draft", "deprecated", "superseded", "archived"]

[crawl]
skip_dirs = ["target", "node_modules", ".git"]

[[repos]]
slug = "alpha"
tier = "primary"
repo_path = "repos/alpha"
status_file = "repos/alpha/planning/status.md"
cache_doc = "docs/projects/alpha.md"
heading = "Alpha"
"#;
    fs::write(root.join("brain.toml"), toml.as_bytes()).unwrap();
}

/// A brain-scoped `state.json` with no leaf repos and no corpus data at
/// all — enough for `attention_queue` to run cleanly and find nothing.
fn write_empty_brain_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "hq",
        "kind": "brain",
        "updated": "2026-06-29",
        "focus": { "now": [], "next": [], "blocked": [] },
        "repos": [],
        "cross_repo": []
    });
    write_json(root, "planning/state.json", &state);
}

/// An HQ brain `state.json` carrying one item in each of the four
/// top-level Attention lanes:
/// - **carryover** (Hot sub-lane — `priority: 1`, ancient `created`)
/// - **aging backlog** (`status: "idea"`, ancient `created`)
/// - **orphaned capture** (`status: "idea"`, `origin.type: "capture"`)
///
/// A `knowledge.md` beside the same `planning/` dir supplies the fourth
/// (**stale distilled knowledge**) lane.
fn write_all_four_lanes_brain_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "hq",
        "kind": "brain",
        "updated": "2026-06-29",
        "focus": { "now": [], "next": [], "blocked": [] },
        "repos": [],
        "cross_repo": [],
        "carryover": [
            {
                "slug": "hq-hot-thing",
                "scope": { "repo": "hq" },
                "kind": "known_issue",
                "text": "A hot carryover with priority 1.",
                "clears_when": "Never, for this fixture.",
                "created": "2020-01-01",
                "priority": 1
            }
        ],
        "backlog": [
            {
                "slug": "aging-idea",
                "title": "An aging backlog idea",
                "repo": "alpha",
                "type": "feature",
                "status": "idea",
                "created": "2020-01-01"
            },
            {
                "slug": "orphaned-capture",
                "title": "An orphaned capture note",
                "repo": "alpha",
                "type": "chore",
                "status": "idea",
                "created": "2020-01-01",
                "origin": { "type": "capture", "notes": "planning/orphaned-capture/notes.md" }
            }
        ]
    });
    write_json(root, "planning/state.json", &state);

    write_file(
        root,
        "planning/knowledge.md",
        "- **A stale distilled claim.** Body text.\n  \
         source: log.md · date: 2020-01-01 · supersedes: — · freshness: 2020-01-01\n",
    );
}

/// A second carryover entry, `Aging` (no priority, past the default
/// `known_issue` staleness threshold but no unmet `blocks[]`), so ordering
/// tests have a lower-priority item to sort after the `Hot` one.
fn write_two_carryover_priorities_brain_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "hq",
        "kind": "brain",
        "updated": "2026-06-29",
        "focus": { "now": [], "next": [], "blocked": [] },
        "repos": [],
        "cross_repo": [],
        "carryover": [
            {
                "slug": "aging-thing",
                "scope": { "repo": "hq" },
                "kind": "known_issue",
                "text": "An aging carryover, no authored priority.",
                "clears_when": "Never, for this fixture.",
                "created": "2020-01-01"
            },
            {
                "slug": "hot-thing",
                "scope": { "repo": "hq" },
                "kind": "known_issue",
                "text": "A hot carryover with priority 1.",
                "clears_when": "Never, for this fixture.",
                "created": "2020-01-01",
                "priority": 1
            }
        ]
    });
    write_json(root, "planning/state.json", &state);
}

fn run_mev(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_mev"))
        .args(args)
        .output()
        .expect("failed to spawn mev binary")
}

fn parse_payloads(stdout: &[u8]) -> Vec<serde_json::Value> {
    let text = String::from_utf8_lossy(stdout);
    serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("stdout is not a JSON array: {e}\nstdout: {text}"))
}

// ---------------------------------------------------------------------------
// (a) One payload per item, across all four top-level lanes.
// ---------------------------------------------------------------------------

#[test]
fn all_four_lanes_produce_payloads() {
    let dir = temp_dir("all-four-lanes");
    write_brain_toml(&dir);
    write_all_four_lanes_brain_state(&dir);

    let out = run_mev(&["attention-queue", dir.to_str().unwrap()]);
    assert!(
        out.status.success(),
        "attention-queue should exit 0; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let payloads = parse_payloads(&out.stdout);
    assert_eq!(payloads.len(), 4, "expected 4 payloads, got: {payloads:#?}");

    let summaries: Vec<String> = payloads
        .iter()
        .map(|p| p["rendered_summary"].as_str().unwrap().to_string())
        .collect();
    let joined = summaries.join(" || ");

    assert!(
        joined.contains("hq-hot-thing") || joined.contains("hot carryover"),
        "missing carryover-lane item: {joined}"
    );
    assert!(
        joined.contains("aging-idea") || joined.contains("aging backlog idea"),
        "missing backlog-lane item: {joined}"
    );
    assert!(
        joined.contains("orphaned-capture") || joined.contains("orphaned capture note"),
        "missing capture-lane item: {joined}"
    );
    assert!(
        joined.contains("stale distilled claim"),
        "missing distilled-lane item: {joined}"
    );

    // Every payload carries the fields EN.8.A requires, within the 2..=3 /
    // 20-char option caps.
    for p in &payloads {
        assert!(!p["gate_id"].as_str().unwrap().is_empty());
        assert!(!p["rendered_summary"].as_str().unwrap().is_empty());
        assert!(!p["digest"].as_str().unwrap().is_empty());
        let options = p["options"].as_array().unwrap();
        assert!(
            (2..=3).contains(&options.len()),
            "options out of 2..=3 bounds: {options:#?}"
        );
        for opt in options {
            let label = opt["label"].as_str().unwrap();
            assert!(
                label.chars().count() <= 20,
                "label exceeds 20 chars: {label:?}"
            );
        }
    }

    // The distilled-lane payload never offers snooze.
    let distilled = payloads
        .iter()
        .find(|p| {
            p["rendered_summary"]
                .as_str()
                .unwrap()
                .contains("stale distilled claim")
        })
        .expect("distilled payload present");
    let distilled_keys: Vec<&str> = distilled["options"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["key"].as_str().unwrap())
        .collect();
    assert!(
        !distilled_keys.contains(&"snooze"),
        "distilled lane must never offer snooze: {distilled_keys:?}"
    );
}

// ---------------------------------------------------------------------------
// (b) Ordering: hottest first by effective_priority.
// ---------------------------------------------------------------------------

#[test]
fn ordering_is_hottest_first() {
    let dir = temp_dir("ordering");
    write_brain_toml(&dir);
    write_two_carryover_priorities_brain_state(&dir);

    let out = run_mev(&["attention-queue", dir.to_str().unwrap()]);
    assert!(out.status.success());

    let payloads = parse_payloads(&out.stdout);
    assert_eq!(payloads.len(), 2, "expected 2 payloads, got: {payloads:#?}");

    // The P1 ("hot-thing") carryover must be ranked before the unprioritized
    // ("aging-thing") one — hottest (lowest effective_priority number) first.
    let first_summary = payloads[0]["rendered_summary"].as_str().unwrap();
    let second_summary = payloads[1]["rendered_summary"].as_str().unwrap();
    assert!(
        first_summary.contains("hot-thing") || first_summary.contains("hot carryover"),
        "hottest item must sort first; got order: {first_summary:?} then {second_summary:?}"
    );
    assert_eq!(payloads[0]["effective_priority"], serde_json::json!(1));
    assert_eq!(payloads[1]["effective_priority"], serde_json::Value::Null);
}

// ---------------------------------------------------------------------------
// (c) Byte-identical output across two runs on an unchanged corpus.
// ---------------------------------------------------------------------------

#[test]
fn running_twice_yields_byte_identical_output() {
    let dir = temp_dir("stable");
    write_brain_toml(&dir);
    write_all_four_lanes_brain_state(&dir);

    let first = run_mev(&["attention-queue", dir.to_str().unwrap()]);
    let second = run_mev(&["attention-queue", dir.to_str().unwrap()]);

    assert!(first.status.success());
    assert!(second.status.success());
    assert_eq!(
        first.stdout, second.stdout,
        "output must be byte-identical across runs on an unchanged corpus"
    );
}

// ---------------------------------------------------------------------------
// (d) An empty corpus emits `[]` and exits 0.
// ---------------------------------------------------------------------------

#[test]
fn empty_corpus_yields_empty_array_and_exit_zero() {
    let dir = temp_dir("empty");
    write_brain_toml(&dir);
    write_empty_brain_state(&dir);

    let out = run_mev(&["attention-queue", dir.to_str().unwrap()]);
    assert!(
        out.status.success(),
        "empty board must exit 0; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let payloads = parse_payloads(&out.stdout);
    assert!(
        payloads.is_empty(),
        "empty corpus must emit an empty array, got: {payloads:#?}"
    );
}

// ---------------------------------------------------------------------------
// (e) Read-only: no file is written other than --out.
// ---------------------------------------------------------------------------

fn list_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    fn walk(dir: &Path, root: &Path, acc: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, root, acc);
            } else {
                acc.push(path.strip_prefix(root).unwrap().to_path_buf());
            }
        }
    }
    walk(root, root, &mut out);
    out.sort();
    out
}

#[test]
fn writes_no_file_other_than_out() {
    let dir = temp_dir("read-only");
    write_brain_toml(&dir);
    write_all_four_lanes_brain_state(&dir);

    let before = list_files(&dir);

    let out = run_mev(&["attention-queue", dir.to_str().unwrap()]);
    assert!(out.status.success());

    let after = list_files(&dir);
    assert_eq!(
        before, after,
        "attention-queue without --out must write no file"
    );

    // With --out, exactly the named file is created and its content matches
    // what stdout would have produced.
    let out_path = dir.join("queue.json");
    let out2 = run_mev(&[
        "attention-queue",
        dir.to_str().unwrap(),
        "--out",
        out_path.to_str().unwrap(),
    ]);
    assert!(out2.status.success());
    assert!(out2.stdout.is_empty(), "--out must suppress stdout output");

    let written = fs::read_to_string(&out_path).unwrap();
    let written_payloads: Vec<serde_json::Value> =
        serde_json::from_str(written.trim()).expect("--out file must be valid JSON");
    let stdout_payloads = parse_payloads(&out.stdout);
    assert_eq!(
        written_payloads, stdout_payloads,
        "--out content must match what stdout would have produced"
    );

    let after_out = list_files(&dir);
    let mut expected = after.clone();
    expected.push(PathBuf::from("queue.json"));
    expected.sort();
    assert_eq!(
        after_out, expected,
        "--out must write exactly the named file and nothing else"
    );
}
