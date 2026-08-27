//! Integration tests for `mev normalize-op-slugs [--write]` (ticket
//! `MV.ticket.op-slug-rendering-and-sweep`, task 4).
//!
//! Drives the **real binary** rather than the library entry point, mirroring
//! `tests/close_operator_gate.rs`'s fixture shape (two repos, "alpha" and
//! "beta", sharing one operator slug across files) and
//! `tests/set_block_status.rs`'s byte-identical / emit-chain assertions.
//! Covers the four claims task 4's acceptance criteria name: dry-run touches
//! zero files, `--write` renames the slug identically across every repo in
//! one call and re-runs `emit-state`, a collision fixture leaves every file
//! byte-identical (proven by content comparison, not exit code alone), and
//! the linked-worktree refusal fires the same way `set-block-status`'s does.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn temp_dir(tag: &str) -> PathBuf {
    let d = mev::testsupport::unique_temp_dir(&format!("mev-normalize-op-slugs-cli-{tag}"));
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

[[repos]]
slug = "beta"
tier = "primary"
repo_path = "repos/beta"
status_file = "repos/beta/planning/status.md"
cache_doc = "docs/projects/beta.md"
heading = "Beta"
"#;
    fs::write(root.join("brain.toml"), toml.as_bytes()).unwrap();
}

fn write_brain_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "hq",
        "kind": "brain",
        "updated": "2026-06-29",
        "focus": { "now": [], "next": [], "blocked": [] },
        "repos": [
            { "repo": "alpha", "now": [], "next": [], "blocked": [] },
            { "repo": "beta", "now": [], "next": [], "blocked": [] }
        ],
        "cross_repo": [],
        "epics": []
    });
    write_file(
        root,
        "planning/state.json",
        &serde_json::to_string_pretty(&state).unwrap(),
    );
}

fn status_md(title: &str) -> String {
    format!(
        "---\ntype: ProjectStatus\ntitle: {title}\ndescription: {title} status fixture.\n---\n\n# {title}\n"
    )
}

fn write_status_docs(root: &Path) {
    write_file(root, "planning/status.md", &status_md("HQ"));
    write_file(root, "repos/alpha/planning/status.md", &status_md("Alpha"));
    write_file(root, "repos/beta/planning/status.md", &status_md("Beta"));
}

/// A repo's `planning/state.json` with one block whose `depends_on` carries a
/// single `{type:"operator", slug}` edge.
fn repo_state_json(repo: &str, block_id: &str, slug: &str) -> String {
    let state = serde_json::json!({
        "repo": repo,
        "kind": "project",
        "updated": "2026-08-01",
        "focus": { "now": [], "next": [], "blocked": [] },
        "tracks": [
            {
                "title": "Phase 1",
                "blocks": [
                    {
                        "id": block_id,
                        "title": format!("{repo} block A"),
                        "status": "open",
                        "wave": 1,
                        "depends_on": [
                            {
                                "type": "operator",
                                "slug": slug,
                                "exit": "planning/handoff.md",
                                "start": "/begin-session mac-mini"
                            }
                        ]
                    }
                ]
            }
        ]
    });
    serde_json::to_string_pretty(&state).unwrap()
}

/// Fixture (1): both `alpha` and `beta` carry an edge on the SAME stuttering
/// slug (`operator-mac-mini-visit`) -- the multi-repo atomicity case.
fn write_shared_stutter_fixture(root: &Path) {
    write_brain_toml(root);
    write_brain_state(root);
    write_status_docs(root);
    write_file(
        root,
        "repos/alpha/planning/state.json",
        &repo_state_json("alpha", "AL.1.A", "operator-mac-mini-visit"),
    );
    write_file(
        root,
        "repos/beta/planning/state.json",
        &repo_state_json("beta", "BE.1.A", "operator-mac-mini-visit"),
    );
}

/// Fixture (2): `alpha` carries a stuttering slug (`operator-team-a`) that
/// would normalize to `team-a`, while `beta` already carries a DISTINCT,
/// non-stuttering edge already named `team-a` -- the untouched-slug collision
/// this command must refuse on, with no writes at all.
fn write_collision_fixture(root: &Path) {
    write_brain_toml(root);
    write_brain_state(root);
    write_status_docs(root);
    write_file(
        root,
        "repos/alpha/planning/state.json",
        &repo_state_json("alpha", "AL.1.A", "operator-team-a"),
    );
    write_file(
        root,
        "repos/beta/planning/state.json",
        &repo_state_json("beta", "BE.1.A", "team-a"),
    );
}

fn run_mev(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mev"))
        .args(args)
        .arg(root)
        .current_dir(root)
        .output()
        .expect("failed to spawn mev binary")
}

const CORPUS_FILES: [&str; 3] = [
    "planning/state.json",
    "repos/alpha/planning/state.json",
    "repos/beta/planning/state.json",
];

fn snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    CORPUS_FILES
        .iter()
        .map(|rel| {
            let p = root.join(rel);
            (p.clone(), fs::read(&p).unwrap())
        })
        .collect()
}

fn assert_unchanged(before: &[(PathBuf, Vec<u8>)]) {
    for (path, bytes_before) in before {
        let bytes_after = fs::read(path).unwrap();
        assert_eq!(
            &bytes_after,
            bytes_before,
            "{} must be byte-identical",
            path.display()
        );
    }
}

fn stdout_stderr(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn slug_of(root: &Path, rel: &str) -> String {
    let v: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(root.join(rel)).unwrap()).unwrap();
    v["tracks"][0]["blocks"][0]["depends_on"][0]["slug"]
        .as_str()
        .unwrap()
        .to_string()
}

fn run_git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .status()
        .expect("failed to invoke git");
    assert!(status.success(), "git {:?} failed in {:?}", args, dir);
}

// ---------------------------------------------------------------------------
// (1) Dry-run reports the plan for BOTH files and writes neither.
// ---------------------------------------------------------------------------

#[test]
fn dry_run_reports_both_files_and_writes_nothing() {
    let dir = temp_dir("dry-run");
    write_shared_stutter_fixture(&dir);
    let before = snapshot(&dir);

    let out = run_mev(&dir, &["normalize-op-slugs"]);
    assert!(out.status.success(), "{}", stdout_stderr(&out));

    let text = stdout_stderr(&out);
    assert!(
        text.contains("'operator-mac-mini-visit' -> 'mac-mini-visit'"),
        "dry run must report the rename plan: {text}"
    );
    assert!(
        text.contains("alpha") && text.contains("beta"),
        "the plan must name both repos it touches: {text}"
    );
    assert!(
        text.contains("W_EMIT_DRY_RUN") || text.contains("would write"),
        "dry run must report the planned writes: {text}"
    );

    assert_unchanged(&before);
    assert_eq!(
        slug_of(&dir, "repos/alpha/planning/state.json"),
        "operator-mac-mini-visit"
    );
    assert_eq!(
        slug_of(&dir, "repos/beta/planning/state.json"),
        "operator-mac-mini-visit"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// (2) --write renames the slug in BOTH repos in one call, and the emit_state
//     chain runs afterward.
// ---------------------------------------------------------------------------

#[test]
fn write_renames_the_slug_in_both_repos_and_reruns_emit_state() {
    let dir = temp_dir("write-both");
    write_shared_stutter_fixture(&dir);

    let out = run_mev(&dir, &["normalize-op-slugs", "--write"]);
    assert!(out.status.success(), "{}", stdout_stderr(&out));

    assert_eq!(
        slug_of(&dir, "repos/alpha/planning/state.json"),
        "mac-mini-visit",
        "alpha's edge must be renamed"
    );
    assert_eq!(
        slug_of(&dir, "repos/beta/planning/state.json"),
        "mac-mini-visit",
        "beta's edge must be renamed too, in the same call"
    );

    let text = stdout_stderr(&out);
    assert!(
        text.contains("I_EMIT_WROTE"),
        "a successful --write must chain into emit_state, which reports I_EMIT_WROTE \
         for the derived surfaces it regenerates: {text}"
    );

    assert!(
        !dir.join(".mev-emit.lock").exists(),
        "the advisory lock must be released after a successful run"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// (3) A collision between a renamed slug and an existing untouched slug
//     aborts the ENTIRE run with no writes -- proven by byte-for-byte content
//     comparison, not merely the exit code.
// ---------------------------------------------------------------------------

#[test]
fn collision_aborts_with_both_files_byte_identical() {
    let dir = temp_dir("collision");
    write_collision_fixture(&dir);
    let before = snapshot(&dir);

    let out = run_mev(&dir, &["normalize-op-slugs", "--write"]);
    assert!(
        !out.status.success(),
        "a collision must exit non-zero: {}",
        stdout_stderr(&out)
    );
    let text = stdout_stderr(&out);
    assert!(text.contains("E_NORMALIZE_OP_SLUG_COLLISION"), "{text}");

    // Byte-for-byte, not just "the exit code was right" -- neither file may
    // have moved at all.
    assert_unchanged(&before);
    assert_eq!(
        slug_of(&dir, "repos/alpha/planning/state.json"),
        "operator-team-a",
        "alpha's slug must be untouched after a refused collision"
    );
    assert_eq!(
        slug_of(&dir, "repos/beta/planning/state.json"),
        "team-a",
        "beta's slug must be untouched after a refused collision"
    );

    assert!(
        !dir.join(".mev-emit.lock").exists(),
        "a refused run must never leave the lockfile behind"
    );

    let _ = fs::remove_dir_all(&dir);
}

// Also confirm a dry-run over the same collision fixture reports the
// collision without needing --write at all, and still writes nothing.
#[test]
fn collision_is_reported_on_a_dry_run_too() {
    let dir = temp_dir("collision-dry-run");
    write_collision_fixture(&dir);
    let before = snapshot(&dir);

    let out = run_mev(&dir, &["normalize-op-slugs"]);
    let text = stdout_stderr(&out);
    assert!(text.contains("E_NORMALIZE_OP_SLUG_COLLISION"), "{text}");
    assert_unchanged(&before);

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// (4) Linked-worktree refusal fires the same way set-block-status's does --
//     a `--write` invocation whose root path resolves inside a linked git
//     worktree must refuse before touching anything.
// ---------------------------------------------------------------------------

#[test]
fn write_refuses_from_inside_a_linked_worktree() {
    let main_dir = temp_dir("worktree-main");
    write_shared_stutter_fixture(&main_dir);

    run_git(&main_dir, &["init", "-q"]);
    run_git(&main_dir, &["config", "user.email", "test@example.com"]);
    run_git(&main_dir, &["config", "user.name", "Test"]);
    run_git(&main_dir, &["add", "-A"]);
    run_git(&main_dir, &["commit", "-q", "-m", "initial commit"]);

    let worktree_parent = temp_dir("worktree-wt-parent");
    let worktree_path = worktree_parent.join("wt");
    run_git(
        &main_dir,
        &[
            "worktree",
            "add",
            worktree_path.to_str().expect("utf8 path"),
        ],
    );

    let before = snapshot(&worktree_path);

    let out = run_mev(&worktree_path, &["normalize-op-slugs", "--write"]);
    assert!(
        !out.status.success(),
        "must refuse a --write from inside a linked worktree: {}",
        stdout_stderr(&out)
    );
    let text = stdout_stderr(&out);
    assert!(
        text.contains("linked git worktree"),
        "the refusal must name the reason: {text}"
    );

    assert_unchanged(&before);
    assert!(
        !worktree_path.join(".mev-emit.lock").exists(),
        "refusing before any write must never take the lock"
    );

    // A dry-run (no --write) from the same linked worktree is unaffected by
    // this guard -- it is read-only, so it is allowed to proceed.
    let dry = run_mev(&worktree_path, &["normalize-op-slugs"]);
    assert!(dry.status.success(), "{}", stdout_stderr(&dry));
    assert_unchanged(&before);

    let _ = fs::remove_dir_all(&main_dir);
    let _ = fs::remove_dir_all(&worktree_parent);
}
