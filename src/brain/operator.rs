//! `mev close-operator-gate <slug> --exit-verified` — the write that clears an
//! `operator` `depends_on` edge fleet-wide.
//!
//! `OK.ticket.operator-edge-types` gave `{type:"operator"}` its shape; this module is
//! the one sanctioned way to remove it once the human-run session it names is done.
//! `ticket-operator-edge-graph` task 6 renders the gate (its `exit`/`start`); this task
//! is what closes it.
//!
//! # Why `--exit-verified` and not just the slug
//!
//! The operator edge's `exit` field names an artifact whose existence ends the gate —
//! but mev never checks the filesystem for that artifact. Confirming it exists is the
//! entire point of the human step this edge represents, so mev cannot infer it without
//! defeating the gate. `--exit-verified` is the caller's plain assertion that they
//! looked; refusing to run without it (rather than defaulting to some other safe
//! behavior) is what keeps this a *human* gate rather than a formality an agent can
//! step around by omission.
//!
//! # Fleet-wide, not per-block
//!
//! One `slug` can be shared across every block that is waiting on the same session
//! (see `okf_core::state::BlockedBy::Operator`'s doc comment and Task 5's
//! `group_blocked_by_gate` dedup). Closing the gate therefore means removing every
//! `{type:"operator", slug: <slug>}` entry across every loaded `state.json`, in one
//! pass under the same `<root>/.mev-emit.lock` every other authored-state writer takes
//! — never one block at a time.
//!
//! # Unknown slug is an error, not a no-op
//!
//! A `close-operator-gate` call naming a slug that matches nothing in the corpus is
//! almost always a typo (`session-mac-mini` vs `session-mac-mini-2`, e.g.), and a
//! silent success there would look identical to a successful close. `plan_close_operator_gate`
//! refuses with `E_OPERATOR_GATE_UNKNOWN` instead, and — because this command is not
//! dry-run/`--write` shaped like its epic/block siblings, it is verified-or-refused —
//! the CLI driver in `close_operator_gate` reports it as a hard failure.

use okf_core::{BlockedBy, StateFile};

use crate::Diagnostic;
use crate::brain::config::BrainConfig;
use crate::brain::emit::EmitPlan;
use crate::brain::epics::action_for;
use crate::brain::state::StateSource;

/// Diagnostic code for a `close-operator-gate` call missing `--exit-verified`.
pub const E_OPERATOR_GATE_NOT_VERIFIED: &str = "E_OPERATOR_GATE_NOT_VERIFIED";
/// Diagnostic code for a `close-operator-gate` call naming a slug with no matching edge.
pub const E_OPERATOR_GATE_UNKNOWN: &str = "E_OPERATOR_GATE_UNKNOWN";

/// Plan the removal of every `{type:"operator", slug: <slug>}` `depends_on` entry
/// across every loaded file.
///
/// Mutates a working copy of `files`, so the caller's slice — and any dry-run caller —
/// is untouched. Emits at most one [`crate::brain::emit::EmitAction`] per touched
/// file, mirroring [`crate::brain::blocks::plan_set_block_status`] and
/// [`crate::brain::epics::plan_epic_cascade`]'s shape.
///
/// Diagnostics:
/// - [`E_OPERATOR_GATE_UNKNOWN`] — no loaded file carries an operator edge with this
///   slug. Nothing is planned; the caller's corpus is never touched.
///
/// `config` is accepted (unused beyond signature parity with the block/epic
/// planners) so this function's call shape matches its siblings and stays easy to
/// extend if a future check needs the repo registry.
pub fn plan_close_operator_gate(
    slug: &str,
    _config: &BrainConfig,
    files: &[(StateSource, StateFile)],
) -> EmitPlan {
    let mut plan = EmitPlan::default();
    let here = std::path::Path::new(".");

    let is_match =
        |dep: &BlockedBy| matches!(dep, BlockedBy::Operator { slug: s, .. } if s == slug);

    // Work on a copy so a caller that never applies the plan cannot mutate its own
    // corpus, exactly like the block/epic planners.
    let mut work: Vec<(StateSource, StateFile)> = files.to_vec();
    let mut touched: std::collections::BTreeMap<usize, usize> = std::collections::BTreeMap::new();
    let mut found = false;

    for (fi, (_, file)) in files.iter().enumerate() {
        for (ti, track) in file.tracks.iter().enumerate() {
            for (bi, block) in track.blocks.iter().enumerate() {
                let hits = block.depends_on.iter().filter(|d| is_match(d)).count();
                if hits == 0 {
                    continue;
                }
                found = true;
                work[fi].1.tracks[ti].blocks[bi]
                    .depends_on
                    .retain(|d| !is_match(d));
                *touched.entry(fi).or_insert(0) += hits;
            }
        }
    }

    if !found {
        plan.diagnostics.push(Diagnostic::error(
            here,
            E_OPERATOR_GATE_UNKNOWN,
            format!(
                "no depends_on entry with operator slug '{slug}' was found in the loaded corpus \
                 — refusing rather than silently succeeding on what is almost certainly a typo"
            ),
        ));
        return plan;
    }

    for (fi, count) in touched {
        let note = format!("close-operator-gate '{slug}' ({count} edge(s) removed)");
        if let Some(a) = action_for(&work[fi].0, &work[fi].1, note) {
            plan.actions.push(a);
        }
    }

    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain::config::RepoEntry;

    fn file_for(
        dir: &std::path::Path,
        repo: &str,
        blocks: &[(&str, &[&str])],
    ) -> (StateSource, StateFile) {
        let abs_path = dir.join(format!("{repo}-state.json"));

        let block_json: Vec<String> = blocks
            .iter()
            .map(|(id, gate_slugs)| {
                let deps: Vec<String> = gate_slugs
                    .iter()
                    .map(|s| {
                        format!(
                            r#"{{"type":"operator","slug":"{s}","exit":"planning/handoff.md","start":"/begin-session {s}"}}"#
                        )
                    })
                    .collect();
                format!(
                    r#"{{ "id": "{id}", "title": "{id}", "status": "open", "wave": 1, "depends_on": [{}] }}"#,
                    deps.join(",")
                )
            })
            .collect();
        let raw = format!(
            r#"{{ "repo": "{repo}", "kind": "project", "updated": "2026-08-01",
  "focus": {{ "now": [], "next": [], "blocked": [] }},
  "tracks": [{{ "title": "P1", "blocks": [{}] }}] }}"#,
            block_json.join(",\n")
        );
        let file: StateFile = serde_json::from_str(&raw).expect("fixture state.json");

        let mut content = serde_json::to_string_pretty(&file).unwrap();
        content.push('\n');
        std::fs::write(&abs_path, content).unwrap();

        let src = StateSource {
            repo_slug: repo.to_string(),
            abs_path,
            expected_kind: "project",
        };
        (src, file)
    }

    fn config_with(slugs: &[&str]) -> BrainConfig {
        BrainConfig {
            repos: slugs
                .iter()
                .map(|s| RepoEntry {
                    slug: s.to_string(),
                    ..Default::default()
                })
                .collect(),
            ..BrainConfig::default()
        }
    }

    #[test]
    fn removes_the_single_matching_edge_and_plans_one_action() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![file_for(
            dir.path(),
            "mev",
            &[("MV.1.A", &["session-mac-mini"])],
        )];
        let plan = plan_close_operator_gate("session-mac-mini", &config_with(&["mev"]), &files);
        assert!(plan.diagnostics.is_empty(), "{:?}", plan.diagnostics);
        assert_eq!(plan.actions.len(), 1, "expected one action: {plan:?}");
        assert!(
            !plan.actions[0].new_content.contains("session-mac-mini"),
            "the operator entry must be gone: {}",
            plan.actions[0].new_content
        );
    }

    #[test]
    fn unknown_slug_is_an_error_and_plans_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![file_for(dir.path(), "mev", &[("MV.1.A", &[])])];
        let plan = plan_close_operator_gate("no-such-slug", &config_with(&["mev"]), &files);
        assert!(plan.actions.is_empty(), "expected no actions: {plan:?}");
        assert_eq!(plan.diagnostics.len(), 1);
        assert_eq!(plan.diagnostics[0].locator, E_OPERATOR_GATE_UNKNOWN);
    }

    #[test]
    fn one_slug_gating_three_blocks_across_two_repos_all_clear_in_one_call() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![
            file_for(
                dir.path(),
                "mev",
                &[
                    ("MV.1.A", &["session-mac-mini"]),
                    ("MV.1.B", &["session-mac-mini"]),
                ],
            ),
            file_for(dir.path(), "bastion", &[("BA.1.A", &["session-mac-mini"])]),
        ];
        let plan = plan_close_operator_gate(
            "session-mac-mini",
            &config_with(&["mev", "bastion"]),
            &files,
        );
        assert!(plan.diagnostics.is_empty(), "{:?}", plan.diagnostics);
        assert_eq!(
            plan.actions.len(),
            2,
            "one action per touched file (2 files): {plan:?}"
        );
        for action in &plan.actions {
            assert!(!action.new_content.contains("session-mac-mini"));
        }
    }

    #[test]
    fn a_different_slug_on_the_same_block_is_left_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![file_for(
            dir.path(),
            "mev",
            &[("MV.1.A", &["session-mac-mini", "session-other"])],
        )];
        let plan = plan_close_operator_gate("session-mac-mini", &config_with(&["mev"]), &files);
        assert_eq!(plan.actions.len(), 1);
        assert!(plan.actions[0].new_content.contains("session-other"));
        assert!(!plan.actions[0].new_content.contains("session-mac-mini"));
    }
}
