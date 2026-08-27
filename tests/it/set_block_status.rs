//! Integration tests for `mev set-block-status` — the first block-level authored
//! mutation (MV.11.B).
//!
//! These drive the **real binary** rather than the library entry point, because
//! half of what this command promises is process-level: dry-run must leave the
//! corpus byte-identical, and every rejection must exit non-zero. A library-only
//! test cannot observe either.
//!
//! The fixture mirrors `tests/brain_epics.rs`'s two-repo brain (HQ + `alpha`) so
//! the two mutation families are exercised against the same shape.

use std::path::Path;
use std::process::{Command, Output};

const BRAIN_TOML: &str = r#"
[vocab]
layer = ["brain", "engine", "meta"]
status = ["active", "draft", "deprecated", "superseded", "archived"]

[crawl]
skip_dirs = ["target", "node_modules", ".git"]

[[repos]]
slug = "brain"
prefix = "BR"
tier = "_root"
repo_path = "."
status_file = "planning/status.md"
cache_doc = "README.md"
heading = "Company Brain"

[[repos]]
slug = "alpha"
prefix = "AL"
tier = "core"
repo_path = "alpha"
status_file = "alpha/planning/status.md"
cache_doc = "docs/projects/alpha.md"
heading = "Alpha"
"#;

/// Same shape as [`brain`], but `alpha` carries exactly two blocks — `AL.1.A`
/// (seeded at `a_status`) and `AL.1.B` (`open`, depending on `AL.1.A` via a
/// `{type:"block"}` `depends_on` entry). Used by the `wontfix` readiness tests,
/// which need a real dependency edge that `brain`'s flat `(id, status)` shape
/// does not carry.
fn brain_with_dependency(a_status: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    std::fs::create_dir_all(root.join("planning")).unwrap();
    std::fs::create_dir_all(root.join("alpha/planning")).unwrap();
    std::fs::write(root.join("brain.toml"), BRAIN_TOML).unwrap();

    std::fs::write(
        root.join("alpha/planning/state.json"),
        format!(
            r#"{{ "repo": "alpha", "kind": "project", "updated": "2026-08-01",
  "focus": {{ "now": [], "next": [], "blocked": [] }},
  "tracks": [{{ "title": "P1", "blocks": [
    {{ "id": "AL.1.A", "title": "AL.1.A", "status": "{a_status}", "wave": 1 }},
    {{ "id": "AL.1.B", "title": "AL.1.B", "wave": 2,
       "depends_on": [{{ "type": "block", "repo": "alpha", "id": "AL.1.A" }}] }}
  ] }}] }}"#
        ),
    )
    .unwrap();

    std::fs::write(
        root.join("planning/state.json"),
        r#"{ "repo": "brain", "kind": "brain", "updated": "2026-08-01",
  "focus": { "now": [], "next": [], "blocked": [] },
  "tracks": [], "repos": [], "cross_repo": [] }"#,
    )
    .unwrap();

    let fm = "---\ntype: ProjectStatus\ntitle: t\ndescription: d\n---\n\n# S\n";
    std::fs::write(root.join("alpha/planning/status.md"), fm).unwrap();
    std::fs::write(root.join("planning/status.md"), fm).unwrap();
    dir
}

/// Build a two-repo brain whose `alpha` blocks carry the given `(id, status)`
/// pairs. Same shape as the epic-mutation fixture.
fn brain(blocks: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    std::fs::create_dir_all(root.join("planning")).unwrap();
    std::fs::create_dir_all(root.join("alpha/planning")).unwrap();
    std::fs::write(root.join("brain.toml"), BRAIN_TOML).unwrap();

    let block_json: Vec<String> = blocks
        .iter()
        .enumerate()
        .map(|(i, (id, status))| {
            format!(
                r#"{{ "id": "{id}", "title": "{id}", "status": "{status}", "wave": {} }}"#,
                i + 1
            )
        })
        .collect();
    std::fs::write(
        root.join("alpha/planning/state.json"),
        format!(
            r#"{{ "repo": "alpha", "kind": "project", "updated": "2026-08-01",
  "focus": {{ "now": [], "next": [], "blocked": [] }},
  "tracks": [{{ "title": "P1", "blocks": [{}] }}] }}"#,
            block_json.join(",\n")
        ),
    )
    .unwrap();

    std::fs::write(
        root.join("planning/state.json"),
        r#"{ "repo": "brain", "kind": "brain", "updated": "2026-08-01",
  "focus": { "now": [], "next": [], "blocked": [] },
  "tracks": [], "repos": [], "cross_repo": [] }"#,
    )
    .unwrap();

    let fm = "---\ntype: ProjectStatus\ntitle: t\ndescription: d\n---\n\n# S\n";
    std::fs::write(root.join("alpha/planning/status.md"), fm).unwrap();
    std::fs::write(root.join("planning/status.md"), fm).unwrap();
    dir
}

/// Run `mev set-block-status <key> <status> <root> [--write]`.
fn run(root: &Path, key: &str, status: &str, write: bool) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_mev"));
    cmd.arg("set-block-status")
        .arg(key)
        .arg(status)
        .arg(root)
        .current_dir(root);
    if write {
        cmd.arg("--write");
    }
    cmd.output().expect("mev should run")
}

fn alpha_state(root: &Path) -> Vec<u8> {
    std::fs::read(root.join("alpha/planning/state.json")).unwrap()
}

/// Every `state.json` + `status.md` in the fixture, as raw bytes — the whole
/// surface a dry run must leave untouched.
fn corpus_bytes(root: &Path) -> Vec<(String, Vec<u8>)> {
    [
        "alpha/planning/state.json",
        "planning/state.json",
        "alpha/planning/status.md",
        "planning/status.md",
    ]
    .iter()
    .map(|rel| {
        (
            rel.to_string(),
            std::fs::read(root.join(rel)).unwrap_or_default(),
        )
    })
    .collect()
}

fn status_of(root: &Path, id: &str) -> String {
    let v: serde_json::Value = serde_json::from_slice(&alpha_state(root)).unwrap();
    v["tracks"][0]["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["id"] == id)
        .unwrap_or_else(|| panic!("no block {id}"))["status"]
        .as_str()
        .unwrap_or("open")
        .to_string()
}

/// The ids currently sitting in `alpha`'s derived `focus.now` lane.
fn focus_now(root: &Path) -> Vec<String> {
    focus_lane(root, "now")
}

/// The ids currently sitting in `alpha`'s derived `focus.<lane>` lane —
/// `"now"`, `"next"`, `"blocked"`, or `"deferred"`.
fn focus_lane(root: &Path, lane: &str) -> Vec<String> {
    let v: serde_json::Value = serde_json::from_slice(&alpha_state(root)).unwrap();
    v["focus"][lane]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|b| b["id"].as_str().unwrap_or_default().to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn stderr_stdout(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

// ── 1. happy path ────────────────────────────────────────────────────────────

#[test]
fn write_changes_the_block_status_on_disk() {
    let dir = brain(&[("AL.1.A", "open"), ("AL.1.B", "open")]);
    let root = dir.path();

    let out = run(root, "alpha:AL.1.A", "closed", true);
    assert!(out.status.success(), "{}", stderr_stdout(&out));

    assert_eq!(status_of(root, "AL.1.A"), "closed");
    assert_eq!(
        status_of(root, "AL.1.B"),
        "open",
        "only the named block may move"
    );
}

// ── 2. dry-run writes nothing ────────────────────────────────────────────────

#[test]
fn dry_run_reports_the_plan_and_leaves_every_file_byte_identical() {
    let dir = brain(&[("AL.1.A", "open")]);
    let root = dir.path();

    let before = corpus_bytes(root);
    let out = run(root, "alpha:AL.1.A", "closed", false);
    let after = corpus_bytes(root);

    assert!(out.status.success(), "{}", stderr_stdout(&out));
    let text = stderr_stdout(&out);
    assert!(
        text.contains("W_EMIT_DRY_RUN") || text.contains("would write"),
        "dry run should report the planned change: {text}"
    );
    assert_eq!(before, after, "dry run must not touch a single byte");
}

// ── 3. idempotency ───────────────────────────────────────────────────────────

#[test]
fn second_identical_write_is_a_byte_identical_no_op() {
    let dir = brain(&[("AL.1.A", "open")]);
    let root = dir.path();

    let first = run(root, "alpha:AL.1.A", "closed", true);
    assert!(first.status.success(), "{}", stderr_stdout(&first));
    let after_first = corpus_bytes(root);

    let second = run(root, "alpha:AL.1.A", "closed", true);
    assert!(
        second.status.success(),
        "setting a block to the status it already has is success, not an error: {}",
        stderr_stdout(&second)
    );
    let after_second = corpus_bytes(root);

    assert_eq!(
        after_first, after_second,
        "the second write must be a fixed point"
    );
}

#[test]
fn no_op_dry_run_plans_nothing() {
    let dir = brain(&[("AL.1.A", "closed")]);
    let root = dir.path();

    let out = run(root, "alpha:AL.1.A", "closed", false);
    assert!(out.status.success(), "{}", stderr_stdout(&out));
    let text = stderr_stdout(&out);
    assert!(
        !text.contains("W_EMIT_DRY_RUN"),
        "already-at-target must plan zero actions: {text}"
    );
    assert!(text.contains("0 error(s)"), "{text}");
}

// ── 4. rejection paths ───────────────────────────────────────────────────────

#[test]
fn unqualified_block_id_is_rejected_and_writes_nothing() {
    let dir = brain(&[("AL.1.A", "open")]);
    let root = dir.path();
    let before = corpus_bytes(root);

    let out = run(root, "AL.1.A", "closed", true);
    assert!(!out.status.success(), "should exit non-zero");
    assert!(
        stderr_stdout(&out).contains("E_BLOCK_BAD_KEY"),
        "{}",
        stderr_stdout(&out)
    );
    assert_eq!(before, corpus_bytes(root));
}

#[test]
fn blocked_is_rejected_because_it_is_a_derived_lane() {
    let dir = brain(&[("AL.1.A", "open")]);
    let root = dir.path();
    let before = corpus_bytes(root);

    let out = run(root, "alpha:AL.1.A", "blocked", true);
    assert!(
        !out.status.success(),
        "`blocked` must never be authorable: {}",
        stderr_stdout(&out)
    );
    let text = stderr_stdout(&out);
    assert!(text.contains("E_BLOCK_BAD_STATUS"), "{text}");
    assert!(
        text.contains("derived lane"),
        "the message should say why: {text}"
    );
    assert_eq!(before, corpus_bytes(root));
    assert_eq!(status_of(root, "AL.1.A"), "open");
}

#[test]
fn unknown_status_is_rejected_and_writes_nothing() {
    let dir = brain(&[("AL.1.A", "open")]);
    let root = dir.path();
    let before = corpus_bytes(root);

    let out = run(root, "alpha:AL.1.A", "done", true);
    assert!(!out.status.success());
    assert!(
        stderr_stdout(&out).contains("E_BLOCK_BAD_STATUS"),
        "{}",
        stderr_stdout(&out)
    );
    assert_eq!(before, corpus_bytes(root));
}

#[test]
fn unknown_block_is_rejected_and_writes_nothing() {
    let dir = brain(&[("AL.1.A", "open")]);
    let root = dir.path();
    let before = corpus_bytes(root);

    let out = run(root, "alpha:AL.9.Z", "closed", true);
    assert!(!out.status.success());
    assert!(
        stderr_stdout(&out).contains("E_BLOCK_NOT_FOUND"),
        "{}",
        stderr_stdout(&out)
    );
    assert_eq!(before, corpus_bytes(root));
}

#[test]
fn unknown_repo_is_rejected_and_writes_nothing() {
    let dir = brain(&[("AL.1.A", "open")]);
    let root = dir.path();
    let before = corpus_bytes(root);

    let out = run(root, "ghost:AL.1.A", "closed", true);
    assert!(!out.status.success());
    assert!(
        stderr_stdout(&out).contains("E_BLOCK_NOT_FOUND"),
        "{}",
        stderr_stdout(&out)
    );
    assert_eq!(before, corpus_bytes(root));
}

// ── 5. the emit-state ripple ─────────────────────────────────────────────────

#[test]
fn write_reruns_emit_state_so_derived_focus_updates() {
    // `focus.now` is derived as "every block with authored status in_progress",
    // so closing AL.1.A must evict it from the lane — but only if the command
    // actually chains into emit-state rather than just editing the JSON.
    let dir = brain(&[("AL.1.A", "in_progress"), ("AL.1.B", "open")]);
    let root = dir.path();

    // Seed the derived surfaces first so `now` genuinely contains AL.1.A.
    let seed = Command::new(env!("CARGO_BIN_EXE_mev"))
        .arg("emit-state")
        .arg(root)
        .arg("--write")
        .current_dir(root)
        .output()
        .expect("emit-state should run");
    assert!(seed.status.success(), "{}", stderr_stdout(&seed));
    assert!(
        focus_now(root).iter().any(|id| id == "AL.1.A"),
        "precondition: an in_progress block should be in focus.now, got {:?}",
        focus_now(root)
    );

    let out = run(root, "alpha:AL.1.A", "closed", true);
    assert!(out.status.success(), "{}", stderr_stdout(&out));

    assert_eq!(status_of(root, "AL.1.A"), "closed");
    assert!(
        !focus_now(root).iter().any(|id| id == "AL.1.A"),
        "a closed block must have been evicted from the derived focus.now lane by the \
         chained emit-state; got {:?}",
        focus_now(root)
    );
}

// ── 6. `wontfix` — terminal block status ─────────────────────────────────────

#[test]
fn wontfix_status_round_trips_and_is_a_fixed_point() {
    let dir = brain(&[("AL.1.A", "open")]);
    let root = dir.path();

    let out = run(root, "alpha:AL.1.A", "wontfix", true);
    assert!(out.status.success(), "{}", stderr_stdout(&out));
    assert_eq!(status_of(root, "AL.1.A"), "wontfix");

    // Setting it again is a no-op success, exactly like every other authored status.
    let before = corpus_bytes(root);
    let again = run(root, "alpha:AL.1.A", "wontfix", true);
    assert!(again.status.success(), "{}", stderr_stdout(&again));
    assert_eq!(
        before,
        corpus_bytes(root),
        "already-at-target wontfix must be a byte-identical fixed point"
    );
}

#[test]
fn wontfix_target_unblocks_its_dependent() {
    // AL.1.B depends_on AL.1.A. Seed AL.1.A as open so AL.1.B starts genuinely
    // blocked, then move AL.1.A to wontfix and confirm AL.1.B is derived ready
    // (focus.next) rather than blocked — terminal-for-readiness, exactly like closed.
    let dir = brain_with_dependency("open");
    let root = dir.path();

    let seed = Command::new(env!("CARGO_BIN_EXE_mev"))
        .arg("emit-state")
        .arg(root)
        .arg("--write")
        .current_dir(root)
        .output()
        .expect("emit-state should run");
    assert!(seed.status.success(), "{}", stderr_stdout(&seed));
    assert!(
        focus_lane(root, "blocked").iter().any(|id| id == "AL.1.B"),
        "precondition: AL.1.B should start blocked on open AL.1.A, got {:?}",
        focus_lane(root, "blocked")
    );

    let out = run(root, "alpha:AL.1.A", "wontfix", true);
    assert!(out.status.success(), "{}", stderr_stdout(&out));
    assert_eq!(status_of(root, "AL.1.A"), "wontfix");

    assert!(
        !focus_lane(root, "blocked").iter().any(|id| id == "AL.1.B"),
        "AL.1.B must no longer be blocked once its dependency is wontfix: {:?}",
        focus_lane(root, "blocked")
    );
    assert!(
        focus_lane(root, "next").iter().any(|id| id == "AL.1.B"),
        "AL.1.B should now derive as ready (next), got {:?}",
        focus_lane(root, "next")
    );
}
