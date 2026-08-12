//! Block-level authored mutation — the sibling of [`crate::brain::epics`] one
//! level down.
//!
//! [`crate::brain::epics`] mutates an *initiative* (an epic plus its member
//! blocks, as a cascade). This module mutates exactly **one** block's authored
//! `status` and nothing else:
//!
//! | Command | Planner | Effect |
//! |---|---|---|
//! | `mev set-block-status <repo:id> <status>` | [`plan_set_block_status`] | that one block's `status` |
//!
//! Same contract as the epic family: returns an [`EmitPlan`], is dry-run by
//! default at the driver level, and the driver re-runs `emit-state` on apply so
//! the derived surfaces (`focus`, boards, rollups) agree with the new authored
//! value.
//!
//! # Why this is a command and not an API
//!
//! `bastion serve` is **read-only by decision (D25)**, so a "mark this block
//! done" action triggered from bastion-web cannot be written there. mev is the
//! deterministic writer for the brain corpus, so the write lands here; the
//! eventual caller is an engine-rs workflow node shelling out to this CLI on
//! bastion-web's behalf. That node is engine-rs work and is not part of this
//! module's contract — mev only owns the command.
//!
//! # Status only, and never `blocked`
//!
//! The surface is deliberately narrow: no `priority`, no `due`, no generic
//! field setter. A generic setter would push per-field validation to runtime and
//! leave the caller's contract vague.
//!
//! Input is validated against
//! [`crate::brain::state::VALID_TRACK_BLOCK_STATUSES`] — `open` · `in_progress`
//! · `deferred` · `closed` · `wontfix` — and **not** against `VALID_STATUSES`. The latter
//! also contains `"blocked"`, which is a **derived** lane the emitter stamps
//! onto `focus.blocked[]` entries; authoring it onto a `tracks[]` block is
//! exactly what `E_STATE_AUTHORED_BLOCKED` exists to reject. Accepting it here
//! would let this command write a value `validate-brain` immediately fails on.
//!
//! # Keys are always `repo:id`
//!
//! Block ids are only unique *within* a repo, so the key is the same
//! `"{repo_slug}:{block_id}"` form `global_status_map` and `effective_priorities`
//! already use. A bare id is rejected rather than guessed at.

use okf_core::StateFile;

use crate::Diagnostic;
use crate::brain::config::BrainConfig;
use crate::brain::emit::EmitPlan;
use crate::brain::epics::action_for;
use crate::brain::state::{StateSource, VALID_TRACK_BLOCK_STATUSES};

/// Default authored status for a block whose `status` field is absent.
const BLOCK_STATUS_OPEN: &str = "open";

/// Split a `repo:id` block key into its two halves.
///
/// Returns `None` when there is no `':'`, or when either half is empty. Splits
/// on the *first* `':'` so a block id containing a colon still resolves.
fn split_key(key: &str) -> Option<(&str, &str)> {
    let (repo, id) = key.split_once(':')?;
    if repo.is_empty() || id.is_empty() {
        return None;
    }
    Some((repo, id))
}

/// Set exactly one block's authored `status`.
///
/// `key` is a `repo:id` block key; `status` must be one of
/// [`VALID_TRACK_BLOCK_STATUSES`].
///
/// Mutates a working copy of `files`, so the caller's slice is untouched and a
/// dry run cannot leak a mutation. Emits at most one [`crate::brain::emit::EmitAction`]
/// — the single file that owns the block.
///
/// Diagnostics:
/// - `E_BLOCK_BAD_KEY` — the key is not `repo:id`.
/// - `E_BLOCK_BAD_STATUS` — the status is not authorable (this is what rejects
///   the derived-only `"blocked"` lane).
/// - `E_BLOCK_NOT_FOUND` — no loaded state file owns that `repo:id`.
///
/// A block that already carries the target status is a **no-op success**: zero
/// actions, zero diagnostics, exit 0 — matching the family's fixed-point
/// discipline.
pub fn plan_set_block_status(
    key: &str,
    status: &str,
    config: &BrainConfig,
    files: &[(StateSource, StateFile)],
) -> EmitPlan {
    let mut plan = EmitPlan::default();
    let here = std::path::Path::new(".");

    // 1. Key shape. Block ids are only unique within a repo — never guess.
    let Some((repo_slug, block_id)) = split_key(key) else {
        plan.diagnostics.push(Diagnostic::error(
            here,
            "E_BLOCK_BAD_KEY",
            format!(
                "block key '{key}' is not in 'repo:id' form (e.g. 'mev:MV.10.A'); block ids are \
                 only unique within a repo, so an unqualified id is ambiguous and is not guessed"
            ),
        ));
        return plan;
    };

    // 2. Status vocabulary. Deliberately the *authored* list — `blocked` is a
    //    derived lane and must never be written onto a tracks[] block.
    if !VALID_TRACK_BLOCK_STATUSES.contains(&status) {
        let extra = if status == "blocked" {
            " — 'blocked' is a derived lane computed by emit-state from unmet dependencies, never \
             an authored value"
        } else {
            ""
        };
        plan.diagnostics.push(Diagnostic::error(
            here,
            "E_BLOCK_BAD_STATUS",
            format!(
                "'{status}' is not an authorable block status; valid values: {}{extra}",
                VALID_TRACK_BLOCK_STATUSES.join(", ")
            ),
        ));
        return plan;
    }

    // 3. Resolve the block across every loaded file's tracks[].blocks[].
    let mut found: Option<(usize, usize, usize)> = None;
    for (fi, (src, file)) in files.iter().enumerate() {
        if src.repo_slug != repo_slug {
            continue;
        }
        for (ti, track) in file.tracks.iter().enumerate() {
            for (bi, block) in track.blocks.iter().enumerate() {
                if block.id == block_id {
                    found = Some((fi, ti, bi));
                }
            }
        }
    }

    let Some((fi, ti, bi)) = found else {
        let known: Vec<&str> = config.repos.iter().map(|r| r.slug.as_str()).collect();
        let hint = if known.contains(&repo_slug) {
            format!("repo '{repo_slug}' has no block '{block_id}'")
        } else {
            format!(
                "no loaded repo is named '{repo_slug}'; known repos: {}",
                known.join(", ")
            )
        };
        plan.diagnostics.push(Diagnostic::error(
            here,
            "E_BLOCK_NOT_FOUND",
            format!("block '{key}' not found — {hint}"),
        ));
        return plan;
    };

    // 4. Fixed point: already there is success, not an error.
    let current = files[fi].1.tracks[ti].blocks[bi]
        .status
        .as_deref()
        .unwrap_or(BLOCK_STATUS_OPEN);
    if current == status {
        return plan;
    }

    // 5. Mutate a working copy and plan the single write.
    let mut work: Vec<(StateSource, StateFile)> = files.to_vec();
    work[fi].1.tracks[ti].blocks[bi].status = Some(status.to_string());

    let note = format!("set block '{key}' status {current} → {status}");
    if let Some(action) = action_for(&work[fi].0, &work[fi].1, note) {
        plan.actions.push(action);
    }

    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One state file for `repo` carrying `(id, status)` blocks, sourced from a
    /// real temp path so [`action_for`]'s on-disk comparison has something to read.
    ///
    /// Built by deserializing JSON rather than by struct literal: `StateFile` has
    /// no `Default`, and going through serde also guarantees the fixture matches
    /// the real on-disk schema.
    fn file_for(
        dir: &std::path::Path,
        repo: &str,
        blocks: &[(&str, Option<&str>)],
    ) -> (StateSource, StateFile) {
        let abs_path = dir.join(format!("{repo}-state.json"));

        let block_json: Vec<String> = blocks
            .iter()
            .map(|(id, status)| match status {
                Some(s) => {
                    format!(r#"{{ "id": "{id}", "title": "{id}", "status": "{s}", "wave": 1 }}"#)
                }
                None => format!(r#"{{ "id": "{id}", "title": "{id}", "wave": 1 }}"#),
            })
            .collect();
        let raw = format!(
            r#"{{ "repo": "{repo}", "kind": "project", "updated": "2026-08-01",
  "focus": {{ "now": [], "next": [], "blocked": [] }},
  "tracks": [{{ "title": "P1", "blocks": [{}] }}] }}"#,
            block_json.join(",\n")
        );
        let file: StateFile = serde_json::from_str(&raw).expect("fixture state.json");

        // Write the *current* serialization so an unchanged file plans nothing.
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
                .map(|s| crate::brain::config::RepoEntry {
                    slug: s.to_string(),
                    ..Default::default()
                })
                .collect(),
            ..BrainConfig::default()
        }
    }

    /// The diagnostic codes a plan raised, in order. mev carries the code in
    /// `Diagnostic::locator`.
    fn codes(plan: &EmitPlan) -> Vec<String> {
        plan.diagnostics.iter().map(|d| d.locator.clone()).collect()
    }

    #[test]
    fn qualified_key_resolves_and_plans_one_action() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![file_for(dir.path(), "mev", &[("MV.10.A", Some("open"))])];
        let plan = plan_set_block_status("mev:MV.10.A", "closed", &config_with(&["mev"]), &files);
        assert_eq!(plan.actions.len(), 1, "expected one action: {plan:?}");
        assert!(plan.diagnostics.is_empty(), "{:?}", plan.diagnostics);
        assert!(plan.actions[0].new_content.contains("\"closed\""));
    }

    #[test]
    fn bare_id_raises_bad_key_and_plans_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![file_for(dir.path(), "mev", &[("MV.10.A", Some("open"))])];
        let plan = plan_set_block_status("MV.10.A", "closed", &config_with(&["mev"]), &files);
        assert!(plan.actions.is_empty());
        assert_eq!(codes(&plan), vec!["E_BLOCK_BAD_KEY"]);
    }

    #[test]
    fn empty_key_half_raises_bad_key() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![file_for(dir.path(), "mev", &[("MV.10.A", Some("open"))])];
        for key in [":MV.10.A", "mev:", ":"] {
            let plan = plan_set_block_status(key, "closed", &config_with(&["mev"]), &files);
            assert_eq!(codes(&plan), vec!["E_BLOCK_BAD_KEY"], "key {key}");
        }
    }

    #[test]
    fn blocked_is_rejected_because_it_is_derived_only() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![file_for(dir.path(), "mev", &[("MV.10.A", Some("open"))])];
        let plan = plan_set_block_status("mev:MV.10.A", "blocked", &config_with(&["mev"]), &files);
        assert!(plan.actions.is_empty());
        assert_eq!(codes(&plan), vec!["E_BLOCK_BAD_STATUS"]);
        assert!(
            plan.diagnostics[0].message.contains("derived lane"),
            "message should explain why: {}",
            plan.diagnostics[0].message
        );
    }

    #[test]
    fn every_authored_status_is_accepted() {
        let dir = tempfile::tempdir().unwrap();
        for want in ["open", "in_progress", "deferred", "closed"] {
            // Start from a status that is never the target so every case mutates.
            let start = if want == "closed" { "open" } else { "closed" };
            let files = vec![file_for(dir.path(), "mev", &[("MV.10.A", Some(start))])];
            let plan = plan_set_block_status("mev:MV.10.A", want, &config_with(&["mev"]), &files);
            assert!(
                plan.diagnostics.is_empty(),
                "{want}: {:?}",
                plan.diagnostics
            );
            assert_eq!(plan.actions.len(), 1, "{want}");
        }
    }

    #[test]
    fn garbage_status_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![file_for(dir.path(), "mev", &[("MV.10.A", Some("open"))])];
        let plan = plan_set_block_status("mev:MV.10.A", "done", &config_with(&["mev"]), &files);
        assert_eq!(codes(&plan), vec!["E_BLOCK_BAD_STATUS"]);
    }

    #[test]
    fn unknown_block_id_raises_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![file_for(dir.path(), "mev", &[("MV.10.A", Some("open"))])];
        let plan = plan_set_block_status("mev:MV.99.Z", "closed", &config_with(&["mev"]), &files);
        assert!(plan.actions.is_empty());
        assert_eq!(codes(&plan), vec!["E_BLOCK_NOT_FOUND"]);
        assert!(plan.diagnostics[0].message.contains("has no block"));
    }

    #[test]
    fn unknown_repo_raises_not_found_with_known_slugs() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![file_for(dir.path(), "mev", &[("MV.10.A", Some("open"))])];
        let plan = plan_set_block_status(
            "ghost:MV.10.A",
            "closed",
            &config_with(&["mev", "bella"]),
            &files,
        );
        assert_eq!(codes(&plan), vec!["E_BLOCK_NOT_FOUND"]);
        let msg = &plan.diagnostics[0].message;
        assert!(msg.contains("known repos"), "{msg}");
        assert!(msg.contains("bella"), "{msg}");
    }

    #[test]
    fn same_id_in_another_repo_is_not_matched() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![
            file_for(dir.path(), "mev", &[("X.1.A", Some("open"))]),
            file_for(dir.path(), "bella", &[("X.1.A", Some("open"))]),
        ];
        let plan = plan_set_block_status(
            "bella:X.1.A",
            "closed",
            &config_with(&["mev", "bella"]),
            &files,
        );
        assert_eq!(plan.actions.len(), 1);
        assert!(
            plan.actions[0].path.ends_with("bella-state.json"),
            "wrong file: {:?}",
            plan.actions[0].path
        );
    }

    #[test]
    fn no_op_when_already_at_target_status() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![file_for(dir.path(), "mev", &[("MV.10.A", Some("closed"))])];
        let plan = plan_set_block_status("mev:MV.10.A", "closed", &config_with(&["mev"]), &files);
        assert!(plan.actions.is_empty(), "{plan:?}");
        assert!(plan.diagnostics.is_empty(), "{plan:?}");
    }

    #[test]
    fn absent_status_is_treated_as_open_for_the_no_op_check() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![file_for(dir.path(), "mev", &[("MV.10.A", None)])];
        let plan = plan_set_block_status("mev:MV.10.A", "open", &config_with(&["mev"]), &files);
        assert!(plan.actions.is_empty(), "{plan:?}");
        assert!(plan.diagnostics.is_empty(), "{plan:?}");
    }

    #[test]
    fn caller_slice_is_never_mutated() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![file_for(dir.path(), "mev", &[("MV.10.A", Some("open"))])];
        let _ = plan_set_block_status("mev:MV.10.A", "closed", &config_with(&["mev"]), &files);
        assert_eq!(
            files[0].1.tracks[0].blocks[0].status.as_deref(),
            Some("open"),
            "planning must not mutate the caller's corpus"
        );
    }

    #[test]
    fn only_the_owning_file_is_touched() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![
            file_for(dir.path(), "mev", &[("MV.10.A", Some("open"))]),
            file_for(dir.path(), "bella", &[("BE.1.A", Some("open"))]),
        ];
        let plan = plan_set_block_status(
            "mev:MV.10.A",
            "closed",
            &config_with(&["mev", "bella"]),
            &files,
        );
        assert_eq!(plan.actions.len(), 1, "exactly one file may be touched");
        assert!(plan.actions[0].path.ends_with("mev-state.json"));
    }
}
