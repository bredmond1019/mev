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

use okf_core::{ApprovalDep, BlockedBy, OperatorDep, StateFile};

use crate::Diagnostic;
use crate::brain::config::BrainConfig;
use crate::brain::emit::EmitPlan;
use crate::brain::epics::action_for;
use crate::brain::state::StateSource;

/// Diagnostic code for a `close-operator-gate` call missing `--exit-verified`.
pub const E_OPERATOR_GATE_NOT_VERIFIED: &str = "E_OPERATOR_GATE_NOT_VERIFIED";
/// Diagnostic code for a `close-operator-gate` call naming a slug with no matching edge.
pub const E_OPERATOR_GATE_UNKNOWN: &str = "E_OPERATOR_GATE_UNKNOWN";
/// Diagnostic code for `approve`/`reject` calls naming a slug with no matching
/// `approval` edge in the loaded corpus.
pub const E_APPROVAL_UNKNOWN: &str = "E_APPROVAL_UNKNOWN";
/// Diagnostic code for `mev approve <slug> --digest <d>` when `<d>` does not match
/// the stored `digest` on (any of) the matching edge(s) — the alarm, per D71.
pub const E_APPROVAL_DIGEST_MISMATCH: &str = "E_APPROVAL_DIGEST_MISMATCH";

/// Diagnostic code for `mev normalize-op-slugs` refusing the entire run because two
/// distinct current slugs would normalize onto the same target — merging two
/// separate gates into one identity is worse than leaving both stuttering, so the
/// whole run aborts with no writes on either side of the collision.
pub const E_NORMALIZE_OP_SLUG_COLLISION: &str = "E_NORMALIZE_OP_SLUG_COLLISION";

/// Diagnostic code reporting one entry of the computed `normalize-op-slugs` rename
/// plan — old slug, normalized target, edge count, and the repos touched. Pushed
/// once per distinct stuttering slug found in the corpus, dry-run or `--write`
/// alike, so a dry-run caller sees the whole plan up front rather than having to
/// reconstruct it from `apply_plan`'s per-file `W_EMIT_DRY_RUN` notes.
pub const I_NORMALIZE_OP_SLUG_PLAN: &str = "I_NORMALIZE_OP_SLUG_PLAN";

/// Plan `mev normalize-op-slugs [--write]`: rename every `depends_on`
/// `operator`/`approval` edge carrying a stuttering slug
/// ([`okf_core::op_slug_stutters`]) to its normalized target
/// ([`okf_core::normalize_op_slug`]), fleet-wide, atomically per slug.
///
/// Structurally this is [`plan_close_operator_gate`]'s discover-corpus-wide,
/// group-by-slug, mutate-a-working-copy shape, aimed at a rename instead of a
/// removal — see that function's doc comment for why "atomic per slug" matters:
/// one slug can gate several blocks across several repos, and renaming some of
/// its edges but not others would split one gate into two.
///
/// # Collision detection runs before any mutation
///
/// The full rename plan (every distinct stuttering slug found in the corpus,
/// mapped to its normalized target) is computed first. If two *different*
/// current slugs would normalize to the same target, the entire run aborts with
/// [`E_NORMALIZE_OP_SLUG_COLLISION`] and **no action is planned at all** — not
/// even for the non-colliding slugs in the same corpus. Silently merging two
/// distinct gates into one shared identity is a worse outcome than leaving every
/// stuttering slug exactly as it was; a fleet-wide, all-or-nothing refusal is the
/// only behavior a caller can safely rely on.
///
/// A slug carrying a doubled prefix (`operator-operator-x`) normalizes to
/// `operator-x` in one pass, which itself still stutters (see
/// `okf_core::normalize_op_slug`'s doc comment) — that is a legitimate,
/// non-colliding rename here, cleaned up fully by a second `normalize-op-slugs`
/// run rather than being treated as a collision with any other `operator-x`-shaped
/// slug that already exists.
///
/// # Reporting
///
/// One [`I_NORMALIZE_OP_SLUG_PLAN`] diagnostic is pushed per distinct rename,
/// naming the old slug, its target, the total edge count, and the repos touched —
/// this is what makes the plan visible on a dry run, before `apply_plan`'s own
/// per-file `W_EMIT_DRY_RUN` notes.
pub fn plan_normalize_op_slugs(files: &[(StateSource, StateFile)]) -> EmitPlan {
    let mut plan = EmitPlan::default();
    let here = std::path::Path::new(".");

    fn slug_of(dep: &BlockedBy) -> Option<&str> {
        match dep {
            BlockedBy::Operator(OperatorDep { slug, .. }) => Some(slug.as_str()),
            BlockedBy::Approval(ApprovalDep { slug, .. }) => Some(slug.as_str()),
            _ => None,
        }
    }

    // Step 1: discover every distinct stuttering slug in the corpus and its
    // normalized target — read-only pass, nothing mutated yet.
    let mut renames: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    for (_, file) in files {
        for track in &file.tracks {
            for block in &track.blocks {
                for dep in &block.depends_on {
                    let Some(slug) = slug_of(dep) else { continue };
                    if okf_core::op_slug_stutters(slug) {
                        renames
                            .entry(slug.to_string())
                            .or_insert_with(|| okf_core::normalize_op_slug(slug).to_string());
                    }
                }
            }
        }
    }

    if renames.is_empty() {
        return plan;
    }

    // Step 2: collision check. Group the distinct OLD slugs by their normalized
    // target; more than one distinct old slug landing on the same target is the
    // collision this refuses on, before anything is written.
    let mut by_target: std::collections::BTreeMap<&str, Vec<&str>> =
        std::collections::BTreeMap::new();
    for (old, new) in &renames {
        by_target
            .entry(new.as_str())
            .or_default()
            .push(old.as_str());
    }
    let mut collided = false;
    for (target, olds) in &by_target {
        if olds.len() > 1 {
            collided = true;
            plan.diagnostics.push(Diagnostic::error(
                here,
                E_NORMALIZE_OP_SLUG_COLLISION,
                format!(
                    "slugs {olds:?} all normalize to '{target}' — aborting the entire \
                     normalize-op-slugs run with no writes rather than silently merging \
                     distinct gates into one shared identity. Rename one of them by hand first."
                ),
            ));
        }
    }

    // A rename target can also collide with a slug that is NOT itself being
    // renamed (a non-stuttering slug already in use, identifying an unrelated,
    // untouched gate) — e.g. "operator-team-a" and "team-a" both present in the
    // corpus: "operator-team-a" -> "team-a" would silently merge into the
    // existing "team-a" gate. `renames.contains_key(slug)` excludes any slug that
    // is itself scheduled to be renamed away in this same pass (the legitimate
    // double-stutter chain, e.g. "operator-x" existing alongside
    // "operator-operator-x" — "operator-x" is not "untouched", it renames to "x"
    // in this same call, so it is not a collision).
    let mut untouched_slugs: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for (_, file) in files {
        for track in &file.tracks {
            for block in &track.blocks {
                for dep in &block.depends_on {
                    if let Some(slug) = slug_of(dep)
                        && !renames.contains_key(slug)
                    {
                        untouched_slugs.insert(slug);
                    }
                }
            }
        }
    }
    for (old, target) in &renames {
        if untouched_slugs.contains(target.as_str()) {
            collided = true;
            plan.diagnostics.push(Diagnostic::error(
                here,
                E_NORMALIZE_OP_SLUG_COLLISION,
                format!(
                    "'{old}' normalizes to '{target}', which already identifies a distinct, \
                     untouched gate in the corpus — aborting the entire normalize-op-slugs run \
                     with no writes rather than merging them into one shared identity."
                ),
            ));
        }
    }

    if collided {
        return plan;
    }

    // Step 3: mutate a working copy — mirrors plan_close_operator_gate's
    // "work on a copy" contract, so a caller that never applies the plan cannot
    // mutate its own corpus. One pass both rewrites the matching slugs and tallies
    // per-file and per-slug counts for the report below.
    let mut work: Vec<(StateSource, StateFile)> = files.to_vec();
    let mut touched_files: std::collections::BTreeMap<usize, usize> =
        std::collections::BTreeMap::new();
    let mut per_slug: std::collections::BTreeMap<
        String,
        (usize, std::collections::BTreeSet<String>),
    > = std::collections::BTreeMap::new();

    for (fi, (src, file)) in work.iter_mut().enumerate() {
        for track in file.tracks.iter_mut() {
            for block in track.blocks.iter_mut() {
                for dep in block.depends_on.iter_mut() {
                    let slug_field: &mut String = match dep {
                        BlockedBy::Operator(OperatorDep { slug, .. }) => slug,
                        BlockedBy::Approval(ApprovalDep { slug, .. }) => slug,
                        _ => continue,
                    };
                    let Some(target) = renames.get(slug_field.as_str()) else {
                        continue;
                    };
                    let old = slug_field.clone();
                    *slug_field = target.clone();
                    *touched_files.entry(fi).or_insert(0) += 1;
                    let entry = per_slug
                        .entry(old)
                        .or_insert_with(|| (0, std::collections::BTreeSet::new()));
                    entry.0 += 1;
                    entry.1.insert(src.repo_slug.clone());
                }
            }
        }
    }

    // Step 4: report the full plan up front, one diagnostic per distinct rename.
    for (old, target) in &renames {
        let (count, repos) = per_slug
            .get(old)
            .cloned()
            .unwrap_or_else(|| (0, std::collections::BTreeSet::new()));
        let repos_str = repos.into_iter().collect::<Vec<_>>().join(", ");
        plan.diagnostics.push(Diagnostic::warning(
            here,
            I_NORMALIZE_OP_SLUG_PLAN,
            format!("'{old}' -> '{target}' ({count} edge(s) across: {repos_str})"),
        ));
    }

    // Step 5: one EmitAction per touched file, mirroring plan_close_operator_gate.
    for (fi, count) in touched_files {
        let note = format!("normalize-op-slugs ({count} edge(s) renamed)");
        if let Some(a) = action_for(&work[fi].0, &work[fi].1, note) {
            plan.actions.push(a);
        }
    }

    plan
}

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

    let is_match = |dep: &BlockedBy| matches!(dep, BlockedBy::Operator(OperatorDep { slug: s, .. }) if s == slug);

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

/// Plan `mev approve <slug> --digest <d>`: verify `digest` against every matching
/// `{type:"approval", slug: <slug>}` edge's stored digest, then either clear all of
/// them (match) or refuse and change nothing (mismatch).
///
/// Diagnostics:
/// - [`E_APPROVAL_UNKNOWN`] — no loaded file carries an `approval` edge with this
///   slug. Nothing is planned; the caller's corpus is never touched.
/// - [`E_APPROVAL_DIGEST_MISMATCH`] — `digest` does not match the stored digest on
///   at least one matching edge. This is the D71 alarm: a **distinct** error
///   diagnostic (never folded into a generic "refused" message) so a re-queue that
///   looks like a no-op does not silently swallow the disagreement. Nothing is
///   removed and no action is planned — the edge stays unmet, i.e. re-queued rather
///   than executed.
///
/// A mismatch on even one matching edge refuses the whole call rather than clearing
/// the edges that did match: a shared slug is meant to carry one reviewed payload,
/// so a split result (some cleared, some not) would leave the corpus in a state that
/// implies the approval was reviewed twice with two different outcomes.
pub fn plan_approve(
    slug: &str,
    digest: &str,
    _config: &BrainConfig,
    files: &[(StateSource, StateFile)],
) -> EmitPlan {
    let mut plan = EmitPlan::default();
    let here = std::path::Path::new(".");

    let is_match = |dep: &BlockedBy| matches!(dep, BlockedBy::Approval(ApprovalDep { slug: s, .. }) if s == slug);

    let mut work: Vec<(StateSource, StateFile)> = files.to_vec();
    let mut touched: std::collections::BTreeMap<usize, usize> = std::collections::BTreeMap::new();
    let mut found = false;
    let mut mismatched_digest: Option<String> = None;

    for (_, file) in files {
        for track in &file.tracks {
            for block in &track.blocks {
                for dep in &block.depends_on {
                    if !is_match(dep) {
                        continue;
                    }
                    found = true;
                    if let BlockedBy::Approval(ApprovalDep { digest: stored, .. }) = dep
                        && stored != digest
                    {
                        mismatched_digest = Some(stored.clone());
                    }
                }
            }
        }
    }

    if !found {
        plan.diagnostics.push(Diagnostic::error(
            here,
            E_APPROVAL_UNKNOWN,
            format!(
                "no depends_on entry with approval slug '{slug}' was found in the loaded corpus \
                 — refusing rather than silently succeeding on what is almost certainly a typo"
            ),
        ));
        return plan;
    }

    if let Some(stored) = mismatched_digest {
        // D71: the mismatch path must ALARM as well as re-queue. This is a distinct,
        // always-surfaced Error diagnostic — report_doc/apply_plan never filter or
        // suppress diagnostics, so pushing this here is what makes it non-suppressible.
        // No action is planned: the edge is left in place, unmet, which is the re-queue.
        plan.diagnostics.push(Diagnostic::error(
            here,
            E_APPROVAL_DIGEST_MISMATCH,
            format!(
                "approval '{slug}': digest mismatch — passed digest does not match the stored \
                 digest '{stored}' on the reviewed edge. The payload changed since review: the \
                 approval is void and the item re-queues as a fresh decision rather than \
                 executing. This may be legitimate drift or a bug upstream — investigate before \
                 re-approving."
            ),
        ));
        return plan;
    }

    // Verified: remove every matching edge, exactly like close-operator-gate.
    for (fi, (_, file)) in files.iter().enumerate() {
        for (ti, track) in file.tracks.iter().enumerate() {
            for (bi, block) in track.blocks.iter().enumerate() {
                let hits = block.depends_on.iter().filter(|d| is_match(d)).count();
                if hits == 0 {
                    continue;
                }
                work[fi].1.tracks[ti].blocks[bi]
                    .depends_on
                    .retain(|d| !is_match(d));
                *touched.entry(fi).or_insert(0) += hits;
            }
        }
    }

    for (fi, count) in touched {
        let note = format!("approve '{slug}' (digest verified, {count} edge(s) cleared)");
        if let Some(a) = action_for(&work[fi].0, &work[fi].1, note) {
            plan.actions.push(a);
        }
    }

    plan
}

/// Plan `mev reject <slug>`: remove every matching `{type:"approval", slug: <slug>}`
/// edge, regardless of digest, and record the rejection in the write note.
///
/// Diagnostics:
/// - [`E_APPROVAL_UNKNOWN`] — no loaded file carries an `approval` edge with this
///   slug. Nothing is planned; the caller's corpus is never touched.
///
/// Unlike `approve`, `reject` never checks `digest` — rejecting a stale or a fresh
/// payload both end the same way: the decision is over and the edge is gone. The
/// rejection is recorded via the write note (mirrors `close-operator-gate`'s
/// `I_EMIT_WROTE` diagnostic), which is always surfaced by `apply_plan`.
pub fn plan_reject(
    slug: &str,
    _config: &BrainConfig,
    files: &[(StateSource, StateFile)],
) -> EmitPlan {
    let mut plan = EmitPlan::default();
    let here = std::path::Path::new(".");

    let is_match = |dep: &BlockedBy| matches!(dep, BlockedBy::Approval(ApprovalDep { slug: s, .. }) if s == slug);

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
            E_APPROVAL_UNKNOWN,
            format!(
                "no depends_on entry with approval slug '{slug}' was found in the loaded corpus \
                 — refusing rather than silently succeeding on what is almost certainly a typo"
            ),
        ));
        return plan;
    }

    for (fi, count) in touched {
        let note = format!("reject '{slug}' ({count} edge(s) removed, decision recorded)");
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

    // -- plan_normalize_op_slugs ---------------------------------------------

    #[test]
    fn normalize_renames_a_stuttering_slug_in_one_file() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![file_for(
            dir.path(),
            "mev",
            &[("MV.1.A", &["operator-mac-mini-visit"])],
        )];
        let plan = plan_normalize_op_slugs(&files);
        assert!(
            plan.diagnostics
                .iter()
                .all(|d| d.severity != crate::Severity::Error),
            "{:?}",
            plan.diagnostics
        );
        assert!(
            plan.diagnostics
                .iter()
                .any(|d| d.locator == I_NORMALIZE_OP_SLUG_PLAN
                    && d.message
                        .contains("'operator-mac-mini-visit' -> 'mac-mini-visit'")),
            "expected a plan diagnostic naming the rename: {:?}",
            plan.diagnostics
        );
        assert_eq!(plan.actions.len(), 1, "expected one action: {plan:?}");
        assert!(
            plan.actions[0]
                .new_content
                .contains("\"slug\": \"mac-mini-visit\""),
            "{}",
            plan.actions[0].new_content
        );
        assert!(
            !plan.actions[0]
                .new_content
                .contains("\"slug\": \"operator-mac-mini-visit\"")
        );
    }

    #[test]
    fn normalize_renames_the_same_slug_across_two_repos_in_one_pass() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![
            file_for(
                dir.path(),
                "mev",
                &[
                    ("MV.1.A", &["operator-mac-mini-visit"]),
                    ("MV.1.B", &["operator-mac-mini-visit"]),
                ],
            ),
            file_for(
                dir.path(),
                "bastion",
                &[("BA.1.A", &["operator-mac-mini-visit"])],
            ),
        ];
        let plan = plan_normalize_op_slugs(&files);
        assert!(
            plan.diagnostics
                .iter()
                .all(|d| d.severity != crate::Severity::Error),
            "{:?}",
            plan.diagnostics
        );
        assert_eq!(
            plan.actions.len(),
            2,
            "one action per touched file (2 files): {plan:?}"
        );
        for action in &plan.actions {
            assert!(
                !action
                    .new_content
                    .contains("\"slug\": \"operator-mac-mini-visit\"")
            );
            assert!(action.new_content.contains("\"slug\": \"mac-mini-visit\""));
        }
        let mev_action = plan
            .actions
            .iter()
            .find(|a| a.path == files[0].0.abs_path)
            .expect("mev file's action present");
        assert_eq!(
            mev_action
                .new_content
                .matches("\"slug\": \"mac-mini-visit\"")
                .count(),
            2,
            "both MV.1.A and MV.1.B edges renamed in the same file: {}",
            mev_action.new_content
        );
    }

    #[test]
    fn normalize_leaves_a_non_stuttering_slug_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![file_for(
            dir.path(),
            "mev",
            &[("MV.1.A", &["mac-mini-visit"])],
        )];
        let plan = plan_normalize_op_slugs(&files);
        assert!(plan.diagnostics.is_empty(), "{:?}", plan.diagnostics);
        assert!(plan.actions.is_empty(), "expected no actions: {plan:?}");
    }

    #[test]
    fn normalize_aborts_the_whole_run_when_a_target_collides_with_an_existing_untouched_slug() {
        // "operator-team-a" and "team-a" differ only in the operator- prefix --
        // renaming the first would silently merge it into the second's identity.
        let dir = tempfile::tempdir().unwrap();
        let files = vec![
            file_for(dir.path(), "mev", &[("MV.1.A", &["operator-team-a"])]),
            file_for(dir.path(), "bastion", &[("BA.1.A", &["team-a"])]),
        ];
        let before: Vec<String> = files
            .iter()
            .map(|(src, _)| std::fs::read_to_string(&src.abs_path).unwrap())
            .collect();

        let plan = plan_normalize_op_slugs(&files);

        assert!(
            plan.actions.is_empty(),
            "collision must plan no writes at all: {plan:?}"
        );
        assert_eq!(
            plan.diagnostics
                .iter()
                .filter(|d| d.locator == E_NORMALIZE_OP_SLUG_COLLISION)
                .count(),
            1,
            "{:?}",
            plan.diagnostics
        );

        // No file was ever touched by apply_plan (nothing to apply since actions
        // is empty), but also confirm the on-disk bytes are still exactly what
        // they were before planning -- planning must not itself mutate the corpus.
        for ((src, _), before) in files.iter().zip(before.iter()) {
            let after = std::fs::read_to_string(&src.abs_path).unwrap();
            assert_eq!(
                &after, before,
                "file must be byte-identical: {:?}",
                src.abs_path
            );
        }
    }

    #[test]
    fn normalize_does_not_treat_a_double_stutter_chain_as_a_collision() {
        // "operator-operator-x" normalizes in one pass to "operator-x", which
        // itself still stutters (and would be renamed again on a second call).
        // "operator-x" existing in the same corpus is not an untouched slug it
        // merges into -- it is itself being renamed away in this same pass, to
        // "x" -- so this must proceed, not abort.
        let dir = tempfile::tempdir().unwrap();
        let files = vec![file_for(
            dir.path(),
            "mev",
            &[
                ("MV.1.A", &["operator-operator-x"]),
                ("MV.1.B", &["operator-x"]),
            ],
        )];
        let plan = plan_normalize_op_slugs(&files);
        assert!(
            plan.diagnostics
                .iter()
                .all(|d| d.locator != E_NORMALIZE_OP_SLUG_COLLISION),
            "double-stutter chain must not be treated as a collision: {:?}",
            plan.diagnostics
        );
        assert_eq!(plan.actions.len(), 1, "{plan:?}");
        assert!(
            plan.actions[0]
                .new_content
                .contains("\"slug\": \"operator-x\"")
        );
        assert!(plan.actions[0].new_content.contains("\"slug\": \"x\""));
    }
}
