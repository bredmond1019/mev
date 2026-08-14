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

use std::collections::HashSet;
use std::path::PathBuf;

use mev::brain::config::{find_brain_root, load_brain_config};
use mev::brain::state::{
    ApprovalDep, Block, BlockDep, BlockedBy, ExternalDep, OperatorDep, StateFile, StateSource,
    build_state_graph, derive_brain_focus, derive_focus, discover_state_files, load_state,
    tier_scope_for,
};

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

    let (sources, _discovery_diags) = discover_state_files(&root, &config);
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
            let scope = tier_scope_for(file, &config);
            let derived = derive_brain_focus(src, file, &scope, &config, &graph, &files);
            (
                ids(&derived.next),
                ids(&derived.blocked),
                ids(&file.focus.next),
                ids(&file.focus.blocked),
            )
        } else {
            let derived = derive_focus(src, file, &graph, &files);
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

    assert!(
        total_blocks_checked > 0,
        "fleet_regression: compared zero blocks across {} loaded files — the fleet fixture \
         path resolved but nothing was actually derived; this would make the gate vacuously \
         pass, which is worse than not having it",
        files.len()
    );

    assert!(
        mismatches.is_empty(),
        "fleet regression: readiness/focus.next/focus.blocked[] changed for {} block(s) that \
         carry no operator/approval edge — a change here can silently un-block or re-block work \
         fleet-wide:\n{}",
        mismatches.len(),
        mismatches.join("\n")
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
