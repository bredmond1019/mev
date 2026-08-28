//! Integration tests for `MV.ticket.write-verbs-ignore-the-quiesce-lease` Task 3: the
//! CLI-level fixture suite over every corpus-wide write verb's quiesce-lease refusal.
//!
//! `src/brain/lease.rs` already carries unit-level coverage of the refusal rule itself
//! (fleet vs repo scope, self-exemption, staleness, malformed-file skip). This file
//! exercises the *CLI* wiring `src/main.rs` adds at each of its `lock::acquire_lock`
//! call sites: that every write verb in the `derive-state-safely` table actually
//! consults the lease store before writing, refuses with the distinct
//! `E_QUIESCE_LEASE_HELD` diagnostic (never confused with `E_EMIT_LOCK_HELD`), never
//! refuses its own holder, and never wedges on a stale or malformed lease.
//!
//! **Every case below builds its own fixture lease store under a temp dir and passes
//! it via `--lock-dir`.** No test here reads, writes, or depends on the real
//! `.fleet-locks` directory this repo's own lane might be running under.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use mev::brain::history;

// ---------------------------------------------------------------------------
// Fixture plumbing — mirrors tests/it/close_operator_gate.rs / approve_reject.rs /
// epic_lock.rs's shape.
// ---------------------------------------------------------------------------

/// Mirrors `mev::brain::lease`'s private staleness threshold (3h = 10800s). Not
/// exported by the crate (it is an internal implementation constant), so this test
/// file states it independently — same value, different owner, exactly the
/// cross-boundary duplication the module's own doc comment already accepts for
/// `check_lane_agents.py`.
const LEASE_STALE_THRESHOLD_SECONDS: f64 = 180.0 * 60.0;

const REVIEWED_DIGEST: &str = "sha256:abc123";

fn temp_dir(tag: &str) -> PathBuf {
    let d = mev::testsupport::unique_temp_dir(&format!("mev-quiesce-lease-cli-{tag}"));
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

/// One block per verb this suite exercises: a plain block for `set-block-status`, an
/// epic member for the `*-epic` family, an operator-gated block for
/// `close-operator-gate`, and an approval-gated block (carrying [`REVIEWED_DIGEST`])
/// for `approve`/`reject`.
fn write_alpha_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "alpha",
        "kind": "project",
        "updated": "2026-08-27",
        "focus": { "now": [], "next": [], "blocked": [] },
        "tracks": [
            {
                "title": "Phase 1",
                "blocks": [
                    { "id": "AL.1.A", "title": "Block A", "status": "open", "wave": 1 },
                    { "id": "AL.1.B", "title": "Block B", "status": "open", "wave": 1, "epics": ["demo"] },
                    {
                        "id": "AL.1.C",
                        "title": "Block C",
                        "status": "open",
                        "wave": 1,
                        "depends_on": [
                            {
                                "type": "operator",
                                "slug": "op-slug",
                                "exit": "planning/handoff.md",
                                "start": "/begin-session x"
                            }
                        ]
                    },
                    {
                        "id": "AL.1.D",
                        "title": "Block D",
                        "status": "open",
                        "wave": 1,
                        "depends_on": [
                            {
                                "type": "approval",
                                "slug": "appr-slug",
                                "what": "Approve the fixture payload",
                                "digest": REVIEWED_DIGEST
                            }
                        ]
                    }
                ]
            }
        ]
    });
    write_json(root, "repos/alpha/planning/state.json", &state);
}

fn write_brain_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "hq",
        "kind": "brain",
        "updated": "2026-08-27",
        "focus": { "now": [], "next": [], "blocked": [] },
        "repos": [
            { "repo": "alpha", "now": [], "next": [], "blocked": [] }
        ],
        "cross_repo": [],
        "epics": [
            { "slug": "demo", "title": "Demo", "status": "active" }
        ]
    });
    write_json(root, "planning/state.json", &state);
}

fn write_fixture(root: &Path) {
    write_brain_toml(root);
    write_brain_state(root);
    write_alpha_state(root);
}

/// Seeds exactly one revision (seq 1) for `repos/alpha/planning/state.json`, so
/// `state-history --restore 1` has something to restore — mirrors
/// `tests/it/state_history.rs`'s direct use of `history::record_revision`.
fn seed_history_revision(root: &Path) {
    let target = root.join("repos/alpha/planning/state.json");
    let current = fs::read(&target).unwrap();
    history::record_revision(&target, &current).unwrap();
}

// ---------------------------------------------------------------------------
// Lease-fixture plumbing — deliberately separate from `root`: every case passes
// this directory explicitly via `--lock-dir`, so nothing here can ever touch the
// real fleet `.fleet-locks`.
// ---------------------------------------------------------------------------

fn lock_dir_for(tag: &str) -> PathBuf {
    let d = mev::testsupport::unique_temp_dir(&format!("mev-quiesce-lease-lockdir-{tag}"));
    // Deliberately not created here — several cases (missing lock dir) want it absent.
    d
}

fn now_rfc3339() -> String {
    chrono::Local::now().to_rfc3339()
}

fn rfc3339_secs_ago(secs_ago: f64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    let target = now - secs_ago;
    let dt = chrono::DateTime::from_timestamp(target as i64, 0).unwrap();
    dt.to_rfc3339()
}

fn lease_json(
    repo: &str,
    lane: &str,
    agent: &str,
    kind: &str,
    scope: Option<&str>,
    acquired_at: &str,
) -> serde_json::Value {
    let mut v = serde_json::json!({
        "repo": repo,
        "lane": lane,
        "agent": agent,
        "acquired_at": acquired_at,
        "kind": kind,
    });
    if let Some(scope) = scope {
        v["scope"] = serde_json::Value::String(scope.to_string());
    }
    v
}

fn write_lease(lock_dir: &Path, name: &str, contents: &serde_json::Value) -> PathBuf {
    let leases_dir = lock_dir.join("leases");
    fs::create_dir_all(&leases_dir).unwrap();
    let path = leases_dir.join(name);
    fs::write(&path, serde_json::to_string_pretty(contents).unwrap()).unwrap();
    path
}

// ---------------------------------------------------------------------------
// CLI runner and the verb table.
// ---------------------------------------------------------------------------

struct VerbCase {
    /// Human label used in assertion failure messages; not necessarily the exact
    /// CLI subcommand string (`state-history --restore` is two tokens).
    label: &'static str,
    args: &'static [&'static str],
}

/// Every corpus-wide write verb the block record's table covers (11 total: the
/// original 9 plus `state-history --restore` and `normalize-op-slugs --write`,
/// both added to the table by the 2026-08-27 file-list correction on the block
/// record). Positional `path` arguments are omitted everywhere they default to
/// `.` — every case here runs with `current_dir(root)`, so the default resolves
/// to the fixture root exactly as if it had been passed explicitly.
const VERBS: &[VerbCase] = &[
    VerbCase {
        label: "emit-state --write",
        args: &["emit-state", "--write"],
    },
    VerbCase {
        label: "set-block-status",
        args: &["set-block-status", "alpha:AL.1.A", "in_progress", "--write"],
    },
    VerbCase {
        label: "defer-epic",
        args: &["defer-epic", "demo", "--write"],
    },
    VerbCase {
        label: "resume-epic",
        args: &["resume-epic", "demo", "--write"],
    },
    VerbCase {
        label: "complete-epic",
        args: &["complete-epic", "demo", "--write"],
    },
    VerbCase {
        label: "sync-epics",
        args: &["sync-epics", "--write"],
    },
    VerbCase {
        label: "close-operator-gate",
        args: &["close-operator-gate", "op-slug", "--exit-verified"],
    },
    VerbCase {
        label: "approve",
        args: &["approve", "appr-slug", "--digest", REVIEWED_DIGEST],
    },
    VerbCase {
        label: "reject",
        args: &["reject", "appr-slug"],
    },
    VerbCase {
        label: "normalize-op-slugs",
        args: &["normalize-op-slugs", "--write"],
    },
    VerbCase {
        label: "state-history --restore",
        args: &[
            "state-history",
            "repos/alpha/planning/state.json",
            "--restore",
            "1",
        ],
    },
];

fn run_mev(
    root: &Path,
    args: &[&str],
    agent: Option<&str>,
    lock_dir: &Path,
) -> std::process::Output {
    let mut full: Vec<&str> = args.to_vec();
    if let Some(a) = agent {
        full.push("--agent");
        full.push(a);
    }
    let lock_dir_str = lock_dir.to_str().unwrap();
    full.push("--lock-dir");
    full.push(lock_dir_str);

    Command::new(env!("CARGO_BIN_EXE_mev"))
        .args(&full)
        .current_dir(root)
        .output()
        .expect("failed to spawn mev binary")
}

/// The two files every write verb here could plausibly touch: alpha's own leaf
/// `state.json` (all 11 verbs target something under it, directly or via
/// `emit-state`'s chained rewrite) and the HQ rollup `state.json` (every `--write`
/// that chains into `emit_state` regenerates it too).
fn read_tracked_files(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    ["repos/alpha/planning/state.json", "planning/state.json"]
        .iter()
        .map(|rel| {
            let p = root.join(rel);
            (p.clone(), fs::read(&p).unwrap())
        })
        .collect()
}

fn assert_tracked_files_unchanged(before: &[(PathBuf, Vec<u8>)], context: &str) {
    for (path, bytes_before) in before {
        let bytes_after = fs::read(path).unwrap();
        assert_eq!(
            &bytes_after,
            bytes_before,
            "{context}: {} must be byte-unchanged after a quiesce refusal",
            path.display()
        );
    }
}

// ---------------------------------------------------------------------------
// (1) A scope:fleet exclusive lease held by ANOTHER agent refuses every verb in
//     the table, and each refusal writes nothing.
// ---------------------------------------------------------------------------

#[test]
fn fleet_scope_exclusive_lease_refuses_every_verb_and_writes_nothing() {
    for verb in VERBS {
        let root = temp_dir(&format!("fleet-refuse-{}", verb.label.replace(' ', "-")));
        write_fixture(&root);
        seed_history_revision(&root);
        let lock_dir = lock_dir_for(&format!("fleet-refuse-{}", verb.label.replace(' ', "-")));

        let lease_path = write_lease(
            &lock_dir,
            "lease-holder.json",
            &lease_json(
                "engine-rs",
                "engine-rs-e3",
                "agent-other",
                "exclusive",
                Some("fleet"),
                &now_rfc3339(),
            ),
        );

        let before = read_tracked_files(&root);

        let output = run_mev(&root, verb.args, Some("agent-a"), &lock_dir);

        assert!(
            !output.status.success(),
            "{}: must exit non-zero when a fleet-scope exclusive lease is held; status: {:?}, stdout: {}, stderr: {}",
            verb.label,
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("E_QUIESCE_LEASE_HELD"),
            "{}: stderr must carry E_QUIESCE_LEASE_HELD; stderr: {stderr}",
            verb.label
        );
        assert!(
            !stderr.contains("error [E_EMIT_LOCK_HELD]"),
            "{}: a quiesce refusal must be reported under its own code, not E_EMIT_LOCK_HELD — \
             the two are distinct conditions with different remedies (the refusal message may \
             still mention E_EMIT_LOCK_HELD in prose, contrasting the two); stderr: {stderr}",
            verb.label
        );
        assert!(
            stderr.contains("engine-rs-e3"),
            "{}: refusal must name the holding lane; stderr: {stderr}",
            verb.label
        );
        assert!(
            stderr.contains("agent-other"),
            "{}: refusal must name the holding agent; stderr: {stderr}",
            verb.label
        );
        assert!(
            stderr.contains("fleet"),
            "{}: refusal must name the lease's scope; stderr: {stderr}",
            verb.label
        );
        assert!(
            stderr.contains(lease_path.to_str().unwrap()),
            "{}: refusal must name the lease's absolute path; stderr: {stderr}",
            verb.label
        );

        assert_tracked_files_unchanged(&before, verb.label);
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&lock_dir);
    }
}

// ---------------------------------------------------------------------------
// (2) The SAME agent's lease never refuses that agent — every verb proceeds.
// ---------------------------------------------------------------------------

#[test]
fn same_agent_lease_never_refuses_that_agent_for_any_verb() {
    for verb in VERBS {
        let root = temp_dir(&format!("self-exempt-{}", verb.label.replace(' ', "-")));
        write_fixture(&root);
        seed_history_revision(&root);
        let lock_dir = lock_dir_for(&format!("self-exempt-{}", verb.label.replace(' ', "-")));

        write_lease(
            &lock_dir,
            "lease-self.json",
            &lease_json(
                "alpha",
                "alpha-lane",
                "agent-a",
                "exclusive",
                Some("fleet"),
                &now_rfc3339(),
            ),
        );

        let output = run_mev(&root, verb.args, Some("agent-a"), &lock_dir);
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(
            !stderr.contains("E_QUIESCE_LEASE_HELD"),
            "{}: the lease holder's own agent must never be refused by its own lease; stderr: {stderr}",
            verb.label
        );
        assert!(
            output.status.success(),
            "{}: self-exempted call must succeed; status: {:?}, stdout: {}, stderr: {stderr}",
            verb.label,
            output.status,
            String::from_utf8_lossy(&output.stdout)
        );

        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&lock_dir);
    }
}

// ---------------------------------------------------------------------------
// (3) A caller supplying no --agent is refused by any live exclusive lease,
//     including one it might have written itself.
// ---------------------------------------------------------------------------

#[test]
fn unidentified_caller_is_refused_by_any_live_exclusive_lease() {
    let root = temp_dir("no-agent-refused");
    write_fixture(&root);
    let lock_dir = lock_dir_for("no-agent-refused");

    write_lease(
        &lock_dir,
        "lease-someone.json",
        &lease_json(
            "alpha",
            "alpha-lane",
            "some-agent",
            "exclusive",
            Some("fleet"),
            &now_rfc3339(),
        ),
    );

    let before = read_tracked_files(&root);
    let output = run_mev(&root, &["emit-state", "--write"], None, &lock_dir);

    assert!(
        !output.status.success(),
        "no --agent must be refused by a live exclusive lease; status: {:?}",
        output.status
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("E_QUIESCE_LEASE_HELD"),
        "stderr must carry E_QUIESCE_LEASE_HELD; stderr: {stderr}"
    );
    assert_tracked_files_unchanged(&before, "no-agent-refused");

    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&lock_dir);
}

// ---------------------------------------------------------------------------
// (4) A stale lease does not refuse. Positive control required: the identical
//     fixture with a fresh timestamp MUST refuse — proving the store CAN
//     refuse before this case asserts that it does not.
// ---------------------------------------------------------------------------

#[test]
fn stale_lease_does_not_refuse_but_its_fresh_twin_does() {
    let root = temp_dir("stale-non-wedge");
    write_fixture(&root);
    let lock_dir = lock_dir_for("stale-non-wedge");

    // Positive control first: an identical lease with a fresh timestamp refuses.
    write_lease(
        &lock_dir,
        "lease-fresh.json",
        &lease_json(
            "alpha",
            "alpha-lane",
            "agent-other",
            "exclusive",
            Some("fleet"),
            &now_rfc3339(),
        ),
    );
    let fresh_output = run_mev(
        &root,
        &["emit-state", "--write"],
        Some("agent-a"),
        &lock_dir,
    );
    assert!(
        !fresh_output.status.success(),
        "positive control: a fresh exclusive lease must refuse; status: {:?}",
        fresh_output.status
    );
    assert!(
        String::from_utf8_lossy(&fresh_output.stderr).contains("E_QUIESCE_LEASE_HELD"),
        "positive control: refusal must carry E_QUIESCE_LEASE_HELD; stderr: {}",
        String::from_utf8_lossy(&fresh_output.stderr)
    );
    fs::remove_file(lock_dir.join("leases").join("lease-fresh.json")).unwrap();

    // Same shape, but the liveness timestamp is well past the staleness threshold.
    let stale_acquired_at = rfc3339_secs_ago(LEASE_STALE_THRESHOLD_SECONDS + 3600.0);
    write_lease(
        &lock_dir,
        "lease-stale.json",
        &lease_json(
            "alpha",
            "alpha-lane",
            "agent-other",
            "exclusive",
            Some("fleet"),
            &stale_acquired_at,
        ),
    );
    let stale_output = run_mev(
        &root,
        &["emit-state", "--write"],
        Some("agent-a"),
        &lock_dir,
    );
    assert!(
        stale_output.status.success(),
        "a stale exclusive lease must not refuse; status: {:?}, stderr: {}",
        stale_output.status,
        String::from_utf8_lossy(&stale_output.stderr)
    );
    assert!(
        !String::from_utf8_lossy(&stale_output.stderr).contains("E_QUIESCE_LEASE_HELD"),
        "a stale lease must never surface E_QUIESCE_LEASE_HELD; stderr: {}",
        String::from_utf8_lossy(&stale_output.stderr)
    );

    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&lock_dir);
}

// ---------------------------------------------------------------------------
// (5) A scope:repo lease naming an UNRELATED repo does not refuse; the same
//     lease naming THIS repo does. Exercised through the CLI's own repo-identity
//     resolution (`resolve_own_repo`), using `emit-state <path>` with an explicit
//     path under `repos/alpha` so the write's own repo resolves to "alpha".
// ---------------------------------------------------------------------------

#[test]
fn repo_scope_lease_on_unrelated_repo_does_not_refuse_but_same_repo_does() {
    let root = temp_dir("repo-scope");
    write_fixture(&root);
    let lock_dir = lock_dir_for("repo-scope");

    write_lease(
        &lock_dir,
        "lease-unrelated-repo.json",
        &lease_json(
            "engine-rs",
            "engine-rs-lane",
            "agent-other",
            "exclusive",
            Some("repo"),
            &now_rfc3339(),
        ),
    );

    // This write's own repo identity resolves via the explicit `repos/alpha` path,
    // which matches brain.toml's `[[repos]] repo_path = "repos/alpha"` entry.
    let unrelated_output = Command::new(env!("CARGO_BIN_EXE_mev"))
        .args([
            "emit-state",
            "repos/alpha",
            "--write",
            "--agent",
            "agent-a",
            "--lock-dir",
            lock_dir.to_str().unwrap(),
        ])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        unrelated_output.status.success(),
        "a repo-scoped lease on an unrelated repo must not refuse; status: {:?}, stderr: {}",
        unrelated_output.status,
        String::from_utf8_lossy(&unrelated_output.stderr)
    );

    write_lease(
        &lock_dir,
        "lease-same-repo.json",
        &lease_json(
            "alpha",
            "alpha-lane",
            "agent-other",
            "exclusive",
            Some("repo"),
            &now_rfc3339(),
        ),
    );

    let before = read_tracked_files(&root);
    let same_repo_output = Command::new(env!("CARGO_BIN_EXE_mev"))
        .args([
            "emit-state",
            "repos/alpha",
            "--write",
            "--agent",
            "agent-a",
            "--lock-dir",
            lock_dir.to_str().unwrap(),
        ])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        !same_repo_output.status.success(),
        "a repo-scoped lease naming this same repo must refuse; status: {:?}",
        same_repo_output.status
    );
    assert!(
        String::from_utf8_lossy(&same_repo_output.stderr).contains("E_QUIESCE_LEASE_HELD"),
        "stderr must carry E_QUIESCE_LEASE_HELD; stderr: {}",
        String::from_utf8_lossy(&same_repo_output.stderr)
    );
    assert_tracked_files_unchanged(&before, "repo-scope same-repo refusal");

    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&lock_dir);
}

// ---------------------------------------------------------------------------
// (6) A shared lease never refuses, at either scope.
// ---------------------------------------------------------------------------

#[test]
fn shared_lease_never_refuses_at_either_scope() {
    let root = temp_dir("shared-lease");
    write_fixture(&root);
    let lock_dir = lock_dir_for("shared-lease");

    write_lease(
        &lock_dir,
        "lease-shared-fleet.json",
        &lease_json(
            "alpha",
            "alpha-lane",
            "agent-other",
            "shared",
            Some("fleet"),
            &now_rfc3339(),
        ),
    );
    write_lease(
        &lock_dir,
        "lease-shared-repo.json",
        &lease_json(
            "alpha",
            "alpha-lane",
            "agent-other",
            "shared",
            Some("repo"),
            &now_rfc3339(),
        ),
    );

    let output = run_mev(
        &root,
        &["emit-state", "--write"],
        Some("agent-a"),
        &lock_dir,
    );
    assert!(
        output.status.success(),
        "a shared lease must never refuse; status: {:?}, stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains("E_QUIESCE_LEASE_HELD"),
        "a shared lease must never surface E_QUIESCE_LEASE_HELD"
    );

    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&lock_dir);
}

// ---------------------------------------------------------------------------
// (7) A malformed / non-JSON lease file is skipped rather than wedging the
//     write, and a missing lock dir resolves to clear.
// ---------------------------------------------------------------------------

#[test]
fn malformed_lease_file_is_skipped_not_wedged() {
    let root = temp_dir("malformed-lease");
    write_fixture(&root);
    let lock_dir = lock_dir_for("malformed-lease");
    let leases_dir = lock_dir.join("leases");
    fs::create_dir_all(&leases_dir).unwrap();
    fs::write(leases_dir.join("lease-broken.json"), "{ not valid json").unwrap();
    fs::write(
        leases_dir.join("lease-wrong-shape.json"),
        serde_json::to_string(&serde_json::json!({"unexpected": "shape"})).unwrap(),
    )
    .unwrap();
    fs::write(leases_dir.join("readme.txt"), "not a lease").unwrap();

    let output = run_mev(
        &root,
        &["emit-state", "--write"],
        Some("agent-a"),
        &lock_dir,
    );
    assert!(
        output.status.success(),
        "malformed lease files must be skipped, never wedge the write; status: {:?}, stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&lock_dir);
}

#[test]
fn missing_lock_dir_resolves_to_clear() {
    let root = temp_dir("missing-lock-dir");
    write_fixture(&root);
    // Deliberately never created — the whole point of this case.
    let lock_dir = lock_dir_for("missing-lock-dir");
    assert!(!lock_dir.exists());

    let output = run_mev(
        &root,
        &["emit-state", "--write"],
        Some("agent-a"),
        &lock_dir,
    );
    assert!(
        output.status.success(),
        "a missing lock dir must resolve to Clear, never a hold; status: {:?}, stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    let _ = fs::remove_dir_all(&root);
}
