//! Fleet regression gate — ticket-operator-edge-graph, Task 11.
//!
//! This is *the* gate for the ticket: `operator`/`approval` readiness derivation
//! (tasks 1-9) must not silently change readiness, `focus.next`, or `focus.blocked[]`
//! for any block that carries none of the new edge types. A change that un-blocks
//! (or blocks) work fleet-wide by accident is the failure class this repo exists to
//! prevent — see `planning/ticket-operator-edge-graph/tasks.md`'s Testing Strategy.
//!
//! ## What "before/after" means here
//!
//! There is no old binary to compare against in-process. The practical realization of
//! "before/after" is: every real `planning/state.json` in the fleet already carries a
//! `focus` snapshot last written by `mev emit-state --write` *before* this ticket's
//! code changes landed (no block in the live fleet has an `operator`/`approval` edge
//! yet — those variants did not exist to author until `OK.ticket.operator-edge-types`
//! shipped the shapes). So "after" derivation (this build's [`derive_focus`] /
//! [`derive_brain_focus`]) compared against the already-stored `focus` snapshot *is*
//! the before/after comparison, for every block that carries no new edge — read
//! literally, "before" is encoded in the file on disk and "after" is what this binary
//! derives from the same `tracks[]`.
//!
//! Blocks that DO carry a new edge are excluded from the comparison — an operator gate
//! authored on a real block is *expected* to show as blocked in `derive_focus` before
//! the file's stored `focus` snapshot has been regenerated via `mev emit-state
//! --write`, and that expected drift is not this test's concern (`check_focus_drift`
//! covers it, at warning severity, elsewhere).
//!
//! ## Portability
//!
//! This test walks up from the crate root looking for `brain.toml` (the real
//! `agentic-portfolio` HQ root that houses this repo). If it isn't found — e.g. `mev`
//! checked out standalone, outside the fleet — the test prints why and returns rather
//! than failing; the fleet-wide guarantee only means something inside the fleet.
//!
//! ## Concurrent-lane stability check
//!
//! The live corpus is not a fixture — other sessions (orchestrated *or* one-off,
//! `/begin-orchestration`-leased or not) can author a block-status change in some
//! OTHER repo's `state.json` without yet re-running `mev emit-state --write` there,
//! which leaves that repo's own stored `focus.next`/`focus.blocked[]` cache
//! transiently stale relative to its own freshly-authored `tracks[]` — the exact
//! "stale sibling `focus.next`" pattern `derive-state-safely` documents as
//! self-resolving. That is real staleness, correctly detected — but it is not a
//! regression in THIS build's derivation code, which is the only thing this gate
//! exists to catch. Measured 2026-08-29: this test failed three times in one session
//! purely because a concurrent `/begin-orchestration` run (which DOES take a fleet
//! lock lease) was actively closing blocks in `jynx`/`base-template` mid-run — but a
//! lease-based skip would miss the equally-common case of a concurrent *one-off*
//! `/sdlc-task`/`/sdlc-flow` run, which takes no lease at all.
//!
//! The fix layers two independent signals, neither of which depends on the other:
//!
//! 1. **Lease check first, when available.** `base-template/scripts/fleet_concurrency_check.py
//!    status` is queried once. If it reports ANY active lease or lock, the corpus is
//!    known — not merely suspected — to be under active edit by an orchestrated lane
//!    right now, and the result is immediately inconclusive with no need to wait.
//!    This is the fast, authoritative path, but it only covers lease-taking
//!    (`/begin-orchestration`-driven) writers.
//! 2. **Timed double-read as the fallback/backstop.** [`collect_mismatches`] is called
//!    twice, a few seconds apart, regardless of what the lease check found — a
//!    concurrent writer's commits are bursty (think, edit, write — not continuous), so
//!    a short window can coincidentally land between two writes even while a lease is
//!    held (measured 2026-08-29: a stable-across-4s read happened while `jynx` still
//!    held an active lease). If the mismatch set differs between the two reads, the
//!    corpus is observably in motion and the result is inconclusive. Only a mismatch
//!    set that is **identical across both reads, with no lease active either**, is
//!    treated as a real, stable disagreement worth failing on.
//!
//! Neither signal alone is sufficient: leases miss unleased one-off writers; a short
//! timed window can miss a bursty leased writer. Together they cover both.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use mev::brain::config::{BrainConfig, find_brain_root, load_brain_config};
use mev::brain::state::{
    ApprovalDep, Block, BlockDep, BlockedBy, ExternalDep, OperatorDep, StateFile, StateSource,
    build_state_graph, derive_brain_focus, derive_focus, discover_state_files, load_state,
    tier_scope_for,
};

/// How long to wait between the two stability-check reads. Long enough that a
/// concurrent `emit-state --write` (which touches many files in one pass) is very
/// unlikely to land entirely between the two reads and be missed; short enough that
/// this test does not meaningfully slow the suite down.
const STABILITY_CHECK_DELAY: Duration = Duration::from_secs(4);

/// True if this `TrackBlock`'s own `depends_on` carries an `operator` or `approval`
/// entry — the "new edge" the ticket adds meaning to.
fn carries_new_edge(deps: &[BlockedBy]) -> bool {
    deps.iter()
        .any(|d| matches!(d, BlockedBy::Operator(_) | BlockedBy::Approval(_)))
}

/// `"repo:id"` block ids (fleet-wide) whose own `depends_on` carries a new edge type,
/// gathered from every loaded file's `tracks[]` regardless of `kind`.
fn new_edge_block_keys(files: &[(StateSource, StateFile)]) -> HashSet<String> {
    let mut keys = HashSet::new();
    for (src, file) in files {
        for track in &file.tracks {
            for block in &track.blocks {
                if carries_new_edge(&block.depends_on) {
                    keys.insert(format!("{}:{}", src.repo_slug, block.id));
                }
            }
        }
    }
    keys
}

fn ids(blocks: &[Block]) -> HashSet<String> {
    blocks.iter().map(|b| b.id.clone()).collect()
}

/// Query `base-template/scripts/fleet_concurrency_check.py status` for whether ANY
/// repo currently holds an active lease or lock. `true` means the corpus is known to
/// be under active edit by an orchestrated lane right now. Fails OPEN — `false` (i.e.
/// "assume no lease") on anything that goes wrong (script absent, `python3` missing,
/// malformed output, timeout) — a lookup failure here must never make the fleet
/// regression check MORE likely to false-fail, only fall back to the timed check.
fn any_active_fleet_lease(root: &Path) -> bool {
    let script = root
        .join("base-template")
        .join("scripts")
        .join("fleet_concurrency_check.py");
    if !script.is_file() {
        return false;
    }

    let output = std::process::Command::new("python3")
        .arg(&script)
        .arg("status")
        .current_dir(root)
        .output();
    let Ok(output) = output else {
        return false;
    };
    if !output.status.success() {
        return false;
    }

    let Ok(text) = String::from_utf8(output.stdout) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return false;
    };

    let has_entries = |key: &str| {
        value
            .get(key)
            .and_then(|v| v.as_array())
            .is_some_and(|a| !a.is_empty())
    };
    has_entries("active") || has_entries("exclusive_leases")
}

/// One full load-derive-compare pass over the live fleet corpus at `root`. Returns
/// `(total_blocks_checked, mismatches)` — same shape and same derivation logic the
/// test used inline before the stability check was added; factored out purely so it
/// can be called twice.
fn collect_mismatches(root: &Path, config: &BrainConfig) -> (usize, Vec<String>) {
    let (sources, _discovery_diags) = discover_state_files(root, config);
    assert!(
        sources.len() >= 5,
        "fleet_regression: only found {} state.json sources under {} — this looks like a \
         broken/partial checkout, not the real fleet; refusing to run a vacuous gate",
        sources.len(),
        root.display()
    );

    let mut files: Vec<(StateSource, StateFile)> = Vec::new();
    for src in sources {
        match load_state(&src.abs_path) {
            Ok(file) => files.push((src, file)),
            Err(e) => panic!(
                "fleet_regression: failed to load {}: {e:?}",
                src.abs_path.display()
            ),
        }
    }

    let graph = build_state_graph(&files);
    let new_edge_keys = new_edge_block_keys(&files);

    let mut total_blocks_checked = 0usize;
    let mut mismatches: Vec<String> = Vec::new();

    // Snapshot the (src, file) pairs before iterating so we can still pass `&files`
    // (the full corpus) into the derivation calls while walking each file in turn.
    for (src, file) in &files {
        if file.tracks.is_empty() {
            continue;
        }

        // Kind-aware expected derivation — mirrors `check_focus_drift`'s split so this
        // test can never disagree with the validator about which derivation applies.
        let (derived_next, derived_blocked, stored_next, stored_blocked) = if file.kind == "brain" {
            let scope = tier_scope_for(file, config);
            let derived = derive_brain_focus(src, file, &scope, config, &graph, &files);
            (
                ids(&derived.next),
                ids(&derived.blocked),
                ids(&file.focus.next),
                ids(&file.focus.blocked),
            )
        } else {
            let derived = derive_focus(src, file, &graph, &files, None);
            let derived_blocked: HashSet<String> =
                derived.blocked.iter().map(|(id, _)| id.clone()).collect();
            (
                derived.next.into_iter().collect(),
                derived_blocked,
                ids(&file.focus.next),
                ids(&file.focus.blocked),
            )
        };

        // Exclude any block id (in either the derived or stored set) that carries a
        // new edge type in its own `depends_on` — that block's drift is expected and
        // owned by `W_STATE_FOCUS_DRIFT`, not this gate.
        let is_excluded = |id: &str| new_edge_keys.contains(&format!("{}:{}", src.repo_slug, id));

        let filter = |set: &HashSet<String>| -> HashSet<String> {
            set.iter().filter(|id| !is_excluded(id)).cloned().collect()
        };

        let (dn, db, sn, sb) = (
            filter(&derived_next),
            filter(&derived_blocked),
            filter(&stored_next),
            filter(&stored_blocked),
        );

        total_blocks_checked += dn.len() + db.len() + sn.len() + sb.len();

        if dn != sn {
            mismatches.push(format!(
                "{} ({}): focus.next drift — stored={:?} derived={:?}",
                src.repo_slug,
                src.abs_path.display(),
                sorted(&sn),
                sorted(&dn),
            ));
        }
        if db != sb {
            mismatches.push(format!(
                "{} ({}): focus.blocked drift — stored={:?} derived={:?}",
                src.repo_slug,
                src.abs_path.display(),
                sorted(&sb),
                sorted(&db),
            ));
        }
    }

    (total_blocks_checked, mismatches)
}

#[test]
fn fleet_readiness_is_unchanged_for_blocks_without_a_new_edge() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = match find_brain_root(&manifest_dir) {
        Ok(root) => root,
        Err(e) => {
            eprintln!(
                "fleet_regression: skipping — no brain.toml found walking up from {}: {e}",
                manifest_dir.display()
            );
            return;
        }
    };

    let config = load_brain_config(&root.join("brain.toml"))
        .unwrap_or_else(|e| panic!("failed to load {}: {e}", root.join("brain.toml").display()));

    let (total_blocks_checked_1, mismatches_1) = collect_mismatches(&root, &config);
    assert!(
        total_blocks_checked_1 > 0,
        "fleet_regression: compared zero blocks on the first read — the fleet fixture path \
         resolved but nothing was actually derived; this would make the gate vacuously pass, \
         which is worse than not having it"
    );

    let mismatch_set_1: HashSet<&str> = mismatches_1.iter().map(String::as_str).collect();
    if mismatch_set_1.is_empty() {
        return; // clean on the first read — no need to spend the stability-check delay
    }

    if any_active_fleet_lease(&root) {
        eprintln!(
            "fleet_regression: inconclusive — {} mismatch(es) found, but an orchestrated lane \
             currently holds an active fleet lease/lock, so the corpus is known to be under \
             active edit right now. Skipping rather than failing; no need to wait out the \
             timed stability check when we already have direct evidence.\nmismatch(es):\n{}",
            mismatches_1.len(),
            mismatches_1.join("\n"),
        );
        return;
    }

    std::thread::sleep(STABILITY_CHECK_DELAY);

    let (total_blocks_checked_2, mismatches_2) = collect_mismatches(&root, &config);
    assert!(
        total_blocks_checked_2 > 0,
        "fleet_regression: compared zero blocks on the second read, after comparing {} on the \
         first — the corpus disappeared mid-test rather than merely changing",
        total_blocks_checked_1
    );

    let mismatch_set_2: HashSet<&str> = mismatches_2.iter().map(String::as_str).collect();
    if mismatch_set_1 != mismatch_set_2 {
        eprintln!(
            "fleet_regression: inconclusive — the mismatch set changed between two reads {:?} \
             apart, which means the live corpus is being actively edited by something else \
             right now (an orchestrated lane, a one-off /sdlc-task run, or a manual write) \
             rather than exhibiting a stable regression in this build's own derivation code. \
             Skipping rather than failing.\nfirst read ({} mismatch(es)):\n{}\nsecond read ({} \
             mismatch(es)):\n{}",
            STABILITY_CHECK_DELAY,
            mismatches_1.len(),
            mismatches_1.join("\n"),
            mismatches_2.len(),
            mismatches_2.join("\n"),
        );
        return;
    }

    // Identical mismatch set across two reads several seconds apart: not explained by
    // something else editing the files in between. Treat as a real, stable regression.
    panic!(
        "fleet regression: readiness/focus.next/focus.blocked[] changed for {} block(s) that \
         carry no operator/approval edge, STABLE across two reads {:?} apart — a change here \
         can silently un-block or re-block work fleet-wide:\n{}",
        mismatches_2.len(),
        STABILITY_CHECK_DELAY,
        mismatches_2.join("\n")
    );
}

fn sorted(set: &HashSet<String>) -> Vec<String> {
    let mut v: Vec<String> = set.iter().cloned().collect();
    v.sort();
    v
}

/// Focused unit check (no filesystem dependency): a block with an `operator`/`approval`
/// entry in its own `depends_on` is correctly identified as "carries a new edge", while a
/// block with only `block`/`external` deps (or none) is not — this is the exclusion
/// predicate the fleet-wide test above relies on to scope its comparison correctly.
#[test]
fn carries_new_edge_identifies_operator_and_approval_only() {
    let block_only = vec![BlockedBy::Block(BlockDep {
        repo: "alpha".to_string(),
        id: "A.1".to_string(),
        what: None,
    })];
    let external_only = vec![BlockedBy::External(ExternalDep {
        what: "waiting on a vendor".to_string(),
    })];
    let none: Vec<BlockedBy> = vec![];
    let with_operator = vec![BlockedBy::Operator(OperatorDep {
        slug: "op-1".to_string(),
        exit: "artifact exists".to_string(),
        start: "mev close-operator-gate op-1 --exit-verified".to_string(),
        what: Some("gate".to_string()),
    })];
    let with_approval = vec![BlockedBy::Approval(ApprovalDep {
        slug: "ap-1".to_string(),
        what: "ship it".to_string(),
        digest: "sha256:deadbeef".to_string(),
    })];

    assert!(!carries_new_edge(&block_only));
    assert!(!carries_new_edge(&external_only));
    assert!(!carries_new_edge(&none));
    assert!(carries_new_edge(&with_operator));
    assert!(carries_new_edge(&with_approval));
}

// -----------------------------------------------------------------------
// any_active_fleet_lease — fail-open branches, none of which the one
// happy-path run (against the real fleet, in the test above) exercises.
// -----------------------------------------------------------------------

#[cfg(unix)]
fn fake_lease_root(dir: &std::path::Path, script_body: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let script_dir = dir.join("base-template").join("scripts");
    std::fs::create_dir_all(&script_dir).unwrap();
    let script = script_dir.join("fleet_concurrency_check.py");
    std::fs::write(&script, script_body).unwrap();
    let mut perms = std::fs::metadata(&script).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&script, perms).unwrap();
    dir.to_path_buf()
}

#[test]
fn any_active_fleet_lease_false_when_script_absent() {
    let dir = tempfile::tempdir().unwrap();
    // No base-template/scripts/fleet_concurrency_check.py under this root at all.
    assert!(!any_active_fleet_lease(dir.path()));
}

#[cfg(unix)]
#[test]
fn any_active_fleet_lease_false_on_non_zero_exit() {
    let dir = tempfile::tempdir().unwrap();
    let root = fake_lease_root(
        dir.path(),
        "#!/usr/bin/env python3\nimport sys\nsys.exit(1)\n",
    );
    assert!(!any_active_fleet_lease(&root));
}

#[cfg(unix)]
#[test]
fn any_active_fleet_lease_false_on_malformed_json() {
    let dir = tempfile::tempdir().unwrap();
    let root = fake_lease_root(dir.path(), "#!/usr/bin/env python3\nprint('not json')\n");
    assert!(!any_active_fleet_lease(&root));
}

#[cfg(unix)]
#[test]
fn any_active_fleet_lease_false_when_both_arrays_empty() {
    let dir = tempfile::tempdir().unwrap();
    let root = fake_lease_root(
        dir.path(),
        "#!/usr/bin/env python3\nprint('{\"active\": [], \"exclusive_leases\": []}')\n",
    );
    assert!(!any_active_fleet_lease(&root));
}

#[cfg(unix)]
#[test]
fn any_active_fleet_lease_true_when_active_is_non_empty() {
    let dir = tempfile::tempdir().unwrap();
    let root = fake_lease_root(
        dir.path(),
        "#!/usr/bin/env python3\nprint('{\"active\": [\"jynx (native-build)\"], \"exclusive_leases\": []}')\n",
    );
    assert!(any_active_fleet_lease(&root));
}

#[cfg(unix)]
#[test]
fn any_active_fleet_lease_true_when_exclusive_leases_is_non_empty() {
    let dir = tempfile::tempdir().unwrap();
    let root = fake_lease_root(
        dir.path(),
        "#!/usr/bin/env python3\nprint('{\"active\": [], \"exclusive_leases\": [\"jynx (exclusive)\"]}')\n",
    );
    assert!(any_active_fleet_lease(&root));
}
