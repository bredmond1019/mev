//! Fleet-wide, read-only `backlog[]` sweep — `mev backlog`.
//!
//! Mirrors `mev carryover` (see `src/brain/carryover.rs`), pointed at a second
//! container: every discovered `planning/state.json`'s `backlog[]` array
//! instead of `carryover[]`. Unlike `carryover[]`'s `clears_when`, a
//! `backlog[]` entry's `clears_when`/`ready_when` fields are typed
//! `Option<ClearsWhen>` directly on the node (`okf_core::Backlog`) — there is
//! no prose-extraction step, so the evaluator here is a straight match over
//! the typed predicate, reusing `ClearsWhen`/`ClearsWhenPredicate` verbatim
//! (okf-core `OK.ticket.backlog-lifecycle-predicates`).
//!
//! Five lanes ([`BacklogLane`]):
//!   - **Cleared** — `clears_when` is a satisfied predicate: the idea is dead.
//!   - **Ready** — `ready_when` is a satisfied predicate: promote it.
//!   - **Waiting** — `ready_when` is an evaluable predicate that is not yet
//!     satisfied.
//!   - **Aging** — neither predicate resolves to anything evaluable, and the
//!     node is older than `brain.toml`'s `[attention] backlog_days`
//!     (via [`crate::brain::state::backlog_stale_age`]).
//!   - **NotEvaluable** — a prose `clears_when`/`ready_when`, a
//!     `command_exits_zero` predicate without `--allow-exec`, or a
//!     predicate-free node that has not yet aged past the threshold.
//!
//! **Read-only by construction.** There is no disposal or mutation mode
//! anywhere in this module — `mev carryover --dispose` destroyed 12 live
//! entries on 2026-09-02 by mining free-prose predicates and evaluating them
//! as if typed; this verb has no equivalent surface at all. A satisfied
//! `clears_when` is evidence the predicate fired, never proof the underlying
//! idea is actually dead — the human reading `Cleared` still has to look.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use okf_core::{Backlog, ClearsWhen, ClearsWhenPredicate, StateFile, StateSource};

use crate::brain::config::AttentionThresholds;
use crate::brain::state::backlog_stale_age;

/// Which of the five lanes a `backlog[]` entry landed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BacklogLane {
    /// `clears_when` is a satisfied typed predicate — the idea is dead.
    /// Read as "predicate satisfied — verify before acting", never as a
    /// verdict: a satisfied predicate is evidence it fired, not evidence the
    /// finding is actually dead.
    Cleared,
    /// `ready_when` is a satisfied typed predicate — promote it.
    Ready,
    /// `ready_when` is an evaluable typed predicate that is not (yet)
    /// satisfied.
    Waiting,
    /// No predicate resolved to anything evaluable, and the node is older
    /// than `[attention] backlog_days`.
    Aging,
    /// Prose-only predicate, an unresolved `command_exits_zero` without
    /// `--allow-exec`, or a predicate-free node that has not yet aged.
    NotEvaluable,
}

/// Why an entry could not be evaluated to `Cleared`/`Ready`/`Waiting`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BacklogNotEvaluableReason {
    /// Neither `clears_when` nor `ready_when` is set, and the node has not
    /// aged past the threshold (or has no parseable `created`/`reviewed`
    /// anchor at all).
    NoPredicate,
    /// `clears_when` and/or `ready_when` is present but is free prose, not a
    /// typed predicate.
    Prose,
    /// A `command_exits_zero` predicate was present but execution was not
    /// opted in (`--allow-exec`). Never `Cleared`/`Ready`/`Waiting` — an
    /// unrun command is unknown, and unknown must never read as satisfied.
    ExecutionNotAllowed,
}

/// One resolved typed-predicate outcome, used internally to decide a lane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PredicateOutcome {
    Satisfied,
    Unsatisfied,
    NotEvaluable(BacklogNotEvaluableReason),
}

/// The evaluated verdict for a single `backlog[]` entry.
#[derive(Debug, Clone, Serialize)]
pub struct BacklogVerdict {
    pub repo: String,
    pub slug: String,
    pub title: String,
    pub kind: String,
    pub status: String,
    pub created: Option<String>,
    /// `None` when the entry is snoozed or carries no parseable anchor date.
    pub age_days: Option<i64>,
    pub lane: BacklogLane,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<BacklogNotEvaluableReason>,
}

/// The full fleet-wide sweep result.
#[derive(Debug, Clone, Default, Serialize)]
pub struct BacklogReport {
    pub total: usize,
    pub cleared: usize,
    pub ready: usize,
    pub waiting: usize,
    pub aging: usize,
    pub not_evaluable: usize,
    pub entries: Vec<BacklogVerdict>,
}

/// Evaluate a single typed [`ClearsWhen`] against the loaded corpus.
///
/// Only [`ClearsWhenPredicate::BlockClosed`], [`ClearsWhenPredicate::FileExists`],
/// [`ClearsWhenPredicate::FileContains`] and [`ClearsWhenPredicate::CommandExitsZero`]
/// are machine-checkable; [`ClearsWhen::Prose`] is never evaluable.
#[allow(clippy::too_many_arguments)]
fn evaluate_predicate(
    cw: &ClearsWhen,
    status_map: &HashMap<String, Option<String>>,
    brain_root: &Path,
    repo_paths: &HashMap<String, PathBuf>,
    owning_repo: &str,
    allow_exec: bool,
    exec_timeout: std::time::Duration,
) -> PredicateOutcome {
    let predicate = match cw {
        ClearsWhen::Prose(_) => {
            return PredicateOutcome::NotEvaluable(BacklogNotEvaluableReason::Prose);
        }
        ClearsWhen::Predicate(p) => p,
    };

    match predicate {
        ClearsWhenPredicate::BlockClosed { repo, id, .. } => {
            let key = format!("{repo}:{id}");
            let satisfied = matches!(status_map.get(&key), Some(Some(s)) if s == "closed");
            if satisfied {
                PredicateOutcome::Satisfied
            } else {
                PredicateOutcome::Unsatisfied
            }
        }
        ClearsWhenPredicate::FileExists { path, .. } => {
            let satisfied =
                resolve_existing_path(path, brain_root, repo_paths, owning_repo).is_some();
            if satisfied {
                PredicateOutcome::Satisfied
            } else {
                PredicateOutcome::Unsatisfied
            }
        }
        ClearsWhenPredicate::FileContains { path, pattern, .. } => {
            match resolve_existing_path(path, brain_root, repo_paths, owning_repo) {
                Some(resolved) => match std::fs::read(&resolved) {
                    Ok(bytes) => match String::from_utf8(bytes) {
                        Ok(contents) if contents.contains(pattern.as_str()) => {
                            PredicateOutcome::Satisfied
                        }
                        Ok(_) => PredicateOutcome::Unsatisfied,
                        Err(_) => PredicateOutcome::Unsatisfied,
                    },
                    Err(_) => PredicateOutcome::Unsatisfied,
                },
                None => PredicateOutcome::Unsatisfied,
            }
        }
        ClearsWhenPredicate::CommandExitsZero { command, .. } => {
            if !allow_exec {
                return PredicateOutcome::NotEvaluable(
                    BacklogNotEvaluableReason::ExecutionNotAllowed,
                );
            }
            let cwd = repo_paths
                .get(owning_repo)
                .cloned()
                .unwrap_or_else(|| brain_root.to_path_buf());
            match run_command_exits_zero(command, &cwd, exec_timeout) {
                Some(true) => PredicateOutcome::Satisfied,
                _ => PredicateOutcome::Unsatisfied,
            }
        }
    }
}

/// Resolve a path against both the brain root and the owning repo's root,
/// requiring `is_file()` (mirrors `carryover.rs::resolve_existing_path`'s
/// two-root strategy, simplified: this module's callers do not need to
/// distinguish "ambiguous" from "absent" for a lane decision).
fn resolve_existing_path(
    path: &str,
    brain_root: &Path,
    repo_paths: &HashMap<String, PathBuf>,
    owning_repo: &str,
) -> Option<PathBuf> {
    let brain_candidate = brain_root.join(path);
    if brain_candidate.is_file() {
        return Some(brain_candidate);
    }
    if let Some(repo_path) = repo_paths.get(owning_repo) {
        let candidate = repo_path.join(path);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Run a `command_exits_zero` predicate's command with an in-process
/// watchdog (`timeout(1)` does not exist on this macOS shell — see
/// `carryover.rs::command_exit_zero_outcome`, the same strategy). Returns
/// `Some(true)`/`Some(false)` on a real exit, `None` on spawn failure or
/// timeout — never treated as satisfied.
fn run_command_exits_zero(command: &str, cwd: &Path, timeout: std::time::Duration) -> Option<bool> {
    use std::process::Stdio;

    let mut child = std::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status.success()),
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(_) => return None,
        }
    }
}

/// Decide one entry's lane from its `clears_when`/`ready_when` outcomes and
/// its age.
#[allow(clippy::too_many_arguments)]
fn assign_lane(
    item: &Backlog,
    status_map: &HashMap<String, Option<String>>,
    brain_root: &Path,
    repo_paths: &HashMap<String, PathBuf>,
    owning_repo: &str,
    today: chrono::NaiveDate,
    thresholds: &AttentionThresholds,
    allow_exec: bool,
    exec_timeout: std::time::Duration,
) -> (BacklogLane, Option<BacklogNotEvaluableReason>) {
    let clears = item.clears_when.as_ref().map(|cw| {
        evaluate_predicate(
            cw,
            status_map,
            brain_root,
            repo_paths,
            owning_repo,
            allow_exec,
            exec_timeout,
        )
    });

    if let Some(outcome) = clears {
        match outcome {
            PredicateOutcome::Satisfied => return (BacklogLane::Cleared, None),
            PredicateOutcome::NotEvaluable(reason) => {
                return (BacklogLane::NotEvaluable, Some(reason));
            }
            PredicateOutcome::Unsatisfied => {}
        }
    }

    let ready = item.ready_when.as_ref().map(|cw| {
        evaluate_predicate(
            cw,
            status_map,
            brain_root,
            repo_paths,
            owning_repo,
            allow_exec,
            exec_timeout,
        )
    });

    if let Some(outcome) = ready {
        match outcome {
            PredicateOutcome::Satisfied => return (BacklogLane::Ready, None),
            PredicateOutcome::Unsatisfied => return (BacklogLane::Waiting, None),
            PredicateOutcome::NotEvaluable(reason) => {
                return (BacklogLane::NotEvaluable, Some(reason));
            }
        }
    }

    // Neither predicate produced Cleared/Ready/Waiting/NotEvaluable(reason):
    // either both fields are None, or a typed predicate resolved to
    // Unsatisfied with no complementary field to escalate to Waiting.
    if backlog_stale_age(item, today, thresholds).is_some() {
        (BacklogLane::Aging, None)
    } else {
        (
            BacklogLane::NotEvaluable,
            Some(BacklogNotEvaluableReason::NoPredicate),
        )
    }
}

/// Evaluate every `backlog[]` entry across `files` and sort the fleet into
/// the five lanes. Modelled on `carryover::evaluate_carryover`; never
/// mutates anything.
///
/// `status_map` is a pre-built `"{repo}:{id}"` → authored block status
/// lookup (same shape the carryover sweep and `derive_focus` build). `today`
/// is the caller's current date. `repo_filter`, when set, restricts the
/// sweep to one repo's entries (matched against the owning file's
/// `StateSource::repo_slug`). `allow_exec` is the opt-in gate for
/// `CommandExitsZero`; `exec_timeout` bounds its child process.
#[allow(clippy::too_many_arguments)]
pub fn evaluate_backlog(
    files: &[(StateSource, StateFile)],
    status_map: &HashMap<String, Option<String>>,
    brain_root: &Path,
    repo_paths: &HashMap<String, PathBuf>,
    today: chrono::NaiveDate,
    thresholds: &AttentionThresholds,
    repo_filter: Option<&str>,
    allow_exec: bool,
    exec_timeout: std::time::Duration,
) -> BacklogReport {
    let mut report = BacklogReport::default();

    for (src, file) in files {
        if let Some(filter) = repo_filter
            && src.repo_slug != filter
        {
            continue;
        }
        for item in &file.backlog {
            let (lane, reason) = assign_lane(
                item,
                status_map,
                brain_root,
                repo_paths,
                &src.repo_slug,
                today,
                thresholds,
                allow_exec,
                exec_timeout,
            );
            let age_days = backlog_age_days(item, today);
            report.entries.push(BacklogVerdict {
                repo: src.repo_slug.clone(),
                slug: item.slug.clone(),
                title: item.title.clone(),
                kind: item.kind.clone(),
                status: item.status.clone(),
                created: item.created.clone(),
                age_days,
                lane,
                reason,
            });
            match lane {
                BacklogLane::Cleared => report.cleared += 1,
                BacklogLane::Ready => report.ready += 1,
                BacklogLane::Waiting => report.waiting += 1,
                BacklogLane::Aging => report.aging += 1,
                BacklogLane::NotEvaluable => report.not_evaluable += 1,
            }
        }
    }

    report.total = report.entries.len();
    report
}

/// Age in days from `max(created, reviewed)` to `today`, `None` when
/// snoozed or when neither date parses. Reported alongside the lane for
/// context even on lanes where age is not the deciding factor.
fn backlog_age_days(item: &Backlog, today: chrono::NaiveDate) -> Option<i64> {
    use crate::brain::state::parse_state_date;

    if item
        .snoozed_until
        .as_deref()
        .and_then(parse_state_date)
        .is_some_and(|d| today < d)
    {
        return None;
    }
    let created = item.created.as_deref().and_then(parse_state_date);
    let reviewed = item.reviewed.as_deref().and_then(parse_state_date);
    let anchor = match (created, reviewed) {
        (Some(c), Some(r)) => Some(c.max(r)),
        (Some(c), None) => Some(c),
        (None, Some(r)) => Some(r),
        (None, None) => None,
    }?;
    Some((today - anchor).num_days())
}

#[cfg(test)]
mod tests {
    use super::*;
    use okf_core::{Backlog, ClearsWhen, ClearsWhenPredicate, StateFile, StateSource, Track};
    use std::collections::HashMap;

    fn source(repo: &str) -> StateSource {
        StateSource {
            repo_slug: repo.to_string(),
            abs_path: PathBuf::from(format!("/fake/{repo}/planning/state.json")),
            expected_kind: "brain",
        }
    }

    fn base_backlog(slug: &str) -> Backlog {
        Backlog {
            slug: slug.to_string(),
            title: format!("idea {slug}"),
            repo: "hq".to_string(),
            kind: "improvement".to_string(),
            status: "idea".to_string(),
            created: Some("2020-01-01".to_string()),
            ..Default::default()
        }
    }

    fn thresholds() -> AttentionThresholds {
        AttentionThresholds::default()
    }

    fn today() -> chrono::NaiveDate {
        chrono::NaiveDate::from_ymd_opt(2026, 9, 4).unwrap()
    }

    fn empty_maps() -> (HashMap<String, Option<String>>, HashMap<String, PathBuf>) {
        (HashMap::new(), HashMap::new())
    }

    #[test]
    fn satisfied_clears_when_lands_in_cleared() {
        let mut item = base_backlog("dead-idea");
        item.clears_when = Some(ClearsWhen::Predicate(ClearsWhenPredicate::BlockClosed {
            repo: "hq".to_string(),
            id: "HQ.1.A".to_string(),
            note: None,
        }));
        let mut status_map = HashMap::new();
        status_map.insert("hq:HQ.1.A".to_string(), Some("closed".to_string()));
        let repo_paths = HashMap::new();

        let files = vec![(
            source("hq"),
            StateFile {
                backlog: vec![item],
                ..Default::default()
            },
        )];
        let report = evaluate_backlog(
            &files,
            &status_map,
            Path::new("/fake"),
            &repo_paths,
            today(),
            &thresholds(),
            None,
            false,
            std::time::Duration::from_secs(1),
        );
        assert_eq!(report.total, 1);
        assert_eq!(report.cleared, 1);
        assert_eq!(report.entries[0].lane, BacklogLane::Cleared);
    }

    #[test]
    fn satisfied_ready_when_lands_in_ready() {
        let mut item = base_backlog("ready-idea");
        item.ready_when = Some(ClearsWhen::Predicate(ClearsWhenPredicate::BlockClosed {
            repo: "hq".to_string(),
            id: "HQ.2.A".to_string(),
            note: None,
        }));
        let mut status_map = HashMap::new();
        status_map.insert("hq:HQ.2.A".to_string(), Some("closed".to_string()));
        let repo_paths = HashMap::new();

        let files = vec![(
            source("hq"),
            StateFile {
                backlog: vec![item],
                ..Default::default()
            },
        )];
        let report = evaluate_backlog(
            &files,
            &status_map,
            Path::new("/fake"),
            &repo_paths,
            today(),
            &thresholds(),
            None,
            false,
            std::time::Duration::from_secs(1),
        );
        assert_eq!(report.ready, 1);
        assert_eq!(report.entries[0].lane, BacklogLane::Ready);
    }

    #[test]
    fn unsatisfied_ready_when_lands_in_waiting() {
        let mut item = base_backlog("waiting-idea");
        item.ready_when = Some(ClearsWhen::Predicate(ClearsWhenPredicate::BlockClosed {
            repo: "hq".to_string(),
            id: "HQ.3.A".to_string(),
            note: None,
        }));
        let status_map = HashMap::new(); // HQ.3.A not closed / not present
        let repo_paths = HashMap::new();

        let files = vec![(
            source("hq"),
            StateFile {
                backlog: vec![item],
                ..Default::default()
            },
        )];
        let report = evaluate_backlog(
            &files,
            &status_map,
            Path::new("/fake"),
            &repo_paths,
            today(),
            &thresholds(),
            None,
            false,
            std::time::Duration::from_secs(1),
        );
        assert_eq!(report.waiting, 1);
        assert_eq!(report.entries[0].lane, BacklogLane::Waiting);
    }

    #[test]
    fn predicate_free_aged_entry_lands_in_aging() {
        let item = base_backlog("old-idea"); // created 2020-01-01, no predicates
        let (status_map, repo_paths) = empty_maps();

        let files = vec![(
            source("hq"),
            StateFile {
                backlog: vec![item],
                ..Default::default()
            },
        )];
        let report = evaluate_backlog(
            &files,
            &status_map,
            Path::new("/fake"),
            &repo_paths,
            today(),
            &thresholds(),
            None,
            false,
            std::time::Duration::from_secs(1),
        );
        assert_eq!(report.aging, 1);
        assert_eq!(report.entries[0].lane, BacklogLane::Aging);
    }

    #[test]
    fn predicate_free_young_entry_lands_in_not_evaluable() {
        let mut item = base_backlog("fresh-idea");
        item.created = Some(today().format("%Y-%m-%d").to_string());
        let (status_map, repo_paths) = empty_maps();

        let files = vec![(
            source("hq"),
            StateFile {
                backlog: vec![item],
                ..Default::default()
            },
        )];
        let report = evaluate_backlog(
            &files,
            &status_map,
            Path::new("/fake"),
            &repo_paths,
            today(),
            &thresholds(),
            None,
            false,
            std::time::Duration::from_secs(1),
        );
        assert_eq!(report.not_evaluable, 1);
        assert_eq!(report.entries[0].lane, BacklogLane::NotEvaluable);
        assert_eq!(
            report.entries[0].reason,
            Some(BacklogNotEvaluableReason::NoPredicate)
        );
    }

    #[test]
    fn prose_predicate_lands_in_not_evaluable() {
        let mut item = base_backlog("prose-idea");
        item.clears_when = Some(ClearsWhen::Prose("someday when it feels right".to_string()));
        let (status_map, repo_paths) = empty_maps();

        let files = vec![(
            source("hq"),
            StateFile {
                backlog: vec![item],
                ..Default::default()
            },
        )];
        let report = evaluate_backlog(
            &files,
            &status_map,
            Path::new("/fake"),
            &repo_paths,
            today(),
            &thresholds(),
            None,
            false,
            std::time::Duration::from_secs(1),
        );
        assert_eq!(report.not_evaluable, 1);
        assert_eq!(
            report.entries[0].reason,
            Some(BacklogNotEvaluableReason::Prose)
        );
    }

    #[test]
    fn command_exits_zero_without_allow_exec_is_not_evaluable_and_never_runs() {
        let sentinel =
            std::env::temp_dir().join(format!("mev-backlog-sentinel-{}", std::process::id()));
        let _ = std::fs::remove_file(&sentinel);

        let mut item = base_backlog("exec-idea");
        item.clears_when = Some(ClearsWhen::Predicate(
            ClearsWhenPredicate::CommandExitsZero {
                command: format!("touch {}", sentinel.display()),
                note: None,
            },
        ));
        let (status_map, repo_paths) = empty_maps();

        let files = vec![(
            source("hq"),
            StateFile {
                backlog: vec![item],
                ..Default::default()
            },
        )];
        let report = evaluate_backlog(
            &files,
            &status_map,
            Path::new("/fake"),
            &repo_paths,
            today(),
            &thresholds(),
            None,
            /* allow_exec */ false,
            std::time::Duration::from_secs(1),
        );
        assert_eq!(report.not_evaluable, 1);
        assert_eq!(
            report.entries[0].reason,
            Some(BacklogNotEvaluableReason::ExecutionNotAllowed)
        );
        // Positive control on the flag: the command must PROVABLY not have run.
        assert!(
            !sentinel.exists(),
            "command_exits_zero must not run without --allow-exec"
        );
        let _ = std::fs::remove_file(&sentinel);
    }

    #[test]
    fn command_exits_zero_with_allow_exec_runs_and_can_clear() {
        let mut item = base_backlog("exec-idea-2");
        item.clears_when = Some(ClearsWhen::Predicate(
            ClearsWhenPredicate::CommandExitsZero {
                command: "true".to_string(),
                note: None,
            },
        ));
        let (status_map, repo_paths) = empty_maps();
        let real_cwd = std::env::temp_dir();

        let files = vec![(
            source("hq"),
            StateFile {
                backlog: vec![item],
                ..Default::default()
            },
        )];
        let report = evaluate_backlog(
            &files,
            &status_map,
            &real_cwd,
            &repo_paths,
            today(),
            &thresholds(),
            None,
            /* allow_exec */ true,
            std::time::Duration::from_secs(2),
        );
        assert_eq!(report.cleared, 1);
    }

    #[test]
    fn repo_filter_restricts_the_sweep() {
        let item_a = base_backlog("idea-a");
        let item_b = base_backlog("idea-b");
        let (status_map, repo_paths) = empty_maps();

        let files = vec![
            (
                source("mev"),
                StateFile {
                    backlog: vec![item_a],
                    ..Default::default()
                },
            ),
            (
                source("bastion"),
                StateFile {
                    backlog: vec![item_b],
                    ..Default::default()
                },
            ),
        ];
        let report = evaluate_backlog(
            &files,
            &status_map,
            Path::new("/fake"),
            &repo_paths,
            today(),
            &thresholds(),
            Some("mev"),
            false,
            std::time::Duration::from_secs(1),
        );
        assert_eq!(report.total, 1);
        assert_eq!(report.entries[0].repo, "mev");
    }

    #[test]
    fn sweep_never_touches_the_input_files() {
        // A pure read pass: this test exists mainly to document intent —
        // evaluate_backlog takes files by shared reference and returns an
        // owned report, so mutation is impossible at the type level. The
        // fleet-level "hash the corpus before/after" assertion (task
        // acceptance criteria) lives in the CLI integration path.
        let item = base_backlog("untouched");
        let (status_map, repo_paths) = empty_maps();
        let files = vec![(
            source("hq"),
            StateFile {
                backlog: vec![item.clone()],
                ..Default::default()
            },
        )];
        let _ = evaluate_backlog(
            &files,
            &status_map,
            Path::new("/fake"),
            &repo_paths,
            today(),
            &thresholds(),
            None,
            false,
            std::time::Duration::from_secs(1),
        );
        assert_eq!(files[0].1.backlog[0].slug, item.slug);
    }

    #[test]
    fn track_import_is_reachable() {
        // Keep the `Track` import meaningful (used by other tests indirectly
        // through StateFile::default()); guards against an unused-import
        // regression if StateFile's Default impl changes shape.
        let _: Vec<Track> = Vec::new();
    }
}
