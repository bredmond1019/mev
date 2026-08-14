//! `mev check-consumers` — outcome model and pure classifier for the consumer compile gate.
//!
//! Three mev changes have broken a downstream repo invisibly: `okf-core:OK.3.B` (101 sites in
//! mev, 31 in bastion), mev's D58 (broke engine-rs's workspace compile), and
//! `MV.ticket.reconcile-failed-consumer` (2 sites in bastion, `board.rs:660` and
//! `block_graph.rs:414`). Every one lived only in *test code* — a compile-only `cargo build`
//! walks straight past struct literals that only test fixtures construct. `check-consumers`
//! compiles each path-dependent consumer's test targets (`cargo nextest run --no-run
//! --locked`) against the working mev and reports the true outcome.
//!
//! This module owns the [`ConsumerOutcome`] type and [`classify`], the pure verdict function —
//! same discipline as [`crate::brain::conformance::toolchain`], whose header explains why the
//! verdict is kept separate from the process I/O that gathers its inputs: a function that only
//! judges `(exit_code, stdout, stderr, was_dirty)` is directly unit-testable without spawning
//! `cargo` in every test run. The runner that does the spawning (task 2) and the CLI surface
//! (task 3) build on top of this.
//!
//! **The distinction this whole ticket exists for:** exit 101 with compiler diagnostics is
//! [`ConsumerOutcome::Broken`]; a stale lockfile — observed as exit 101 *or* 102 on
//! 2026-08-13, bastion and engine-rs respectively — is [`ConsumerOutcome::LockfileStale`].
//! Exit code alone cannot separate them (both used 101 on different days), so classification
//! reads the **stderr signature**, not the exit code.

use serde::Serialize;

/// The verdict for one consumer's compile-gate run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ConsumerOutcome {
    /// The consumer's test targets compiled cleanly against the working mev.
    Pass,
    /// A genuine type/API break. `errors` names each compiler diagnostic's code and site
    /// (`"E0063 at src/serve/handlers/board.rs:660:9"`) so the report can point at the exact
    /// lines rather than saying "it broke".
    Broken { errors: Vec<String> },
    /// The consumer's `Cargo.lock` is stale relative to its `Cargo.toml` — bookkeeping for
    /// that repo to fix, not evidence that mev broke anything. Must never be reported as
    /// `Broken`; conflating the two is the exact failure mode this ticket exists to prevent.
    LockfileStale,
    /// `git status --porcelain` was non-empty for this consumer. A dirty consumer's result is
    /// not evidence about mev's change either way, so this short-circuits before any other
    /// classification runs.
    SkippedDirty,
    /// The failure did not match a known signature (neither a compiler diagnostic nor the
    /// lockfile-stale message). Reported with `reason` rather than guessed as `Broken` — an
    /// unrecognised failure is not evidence of a type break.
    NotEvaluable { reason: String },
}

/// One consumer's classified result, carrying the consumer's slug (e.g. `"bastion"`,
/// `"engine-rs"`) alongside its [`ConsumerOutcome`].
#[derive(Debug, Clone, Serialize)]
pub struct ConsumerResult {
    pub slug: String,
    pub outcome: ConsumerOutcome,
}

/// The literal cargo message a `--locked` run emits when the lock file is out of date. Cargo's
/// exact wording varies (it is `Caused by:` chained with several other lines), but this
/// substring is stable across the exit-101 (bastion-style dependency resolution failure) and
/// exit-102 ("cannot update the lock file") cases observed on 2026-08-13.
const LOCKFILE_STALE_SIGNATURE: &str = "cannot update the lock file";

/// Classify a completed (or short-circuited) consumer run as a [`ConsumerOutcome`]. Pure: no
/// process spawning, no filesystem access — every input is already in hand.
///
/// `was_dirty` is checked first and short-circuits to [`ConsumerOutcome::SkippedDirty`]
/// regardless of `exit_code`: a dirty consumer's compile result says nothing about mev's
/// change, because the consumer's own uncommitted work could be the real cause of any failure.
pub fn classify(exit_code: i32, stdout: &str, stderr: &str, was_dirty: bool) -> ConsumerOutcome {
    if was_dirty {
        return ConsumerOutcome::SkippedDirty;
    }

    if exit_code == 0 {
        return ConsumerOutcome::Pass;
    }

    if stderr.contains(LOCKFILE_STALE_SIGNATURE) {
        return ConsumerOutcome::LockfileStale;
    }

    let errors = extract_compiler_errors(stderr);
    if !errors.is_empty() {
        return ConsumerOutcome::Broken { errors };
    }

    // stdout is accepted as an input (nextest sometimes routes diagnostics there under
    // `--no-run`) but is not currently mined for signatures; kept in the signature so a future
    // signature can read it without reshaping the classifier's interface.
    let _ = stdout;

    ConsumerOutcome::NotEvaluable {
        reason: format!(
            "unrecognised failure: exit code {exit_code}, no known signature (compiler \
             diagnostic or lockfile-stale message) found in stderr"
        ),
    }
}

/// Pull `error[E....]: ...` diagnostics out of rustc/cargo stderr, pairing each with its
/// `--> file:line:col` site when one appears in the following lines. Returns entries like
/// `"E0063 at src/serve/handlers/board.rs:660:9"`, or bare `"E0063"` if no site line was found.
fn extract_compiler_errors(stderr: &str) -> Vec<String> {
    let lines: Vec<&str> = stderr.lines().collect();
    let mut errors = Vec::new();

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        let Some(rest) = trimmed.strip_prefix("error[") else {
            continue;
        };
        let Some(close) = rest.find(']') else {
            continue;
        };
        let code = &rest[..close];

        let site = lines
            .iter()
            .skip(i + 1)
            .take(4)
            .find_map(|follow| follow.trim_start().strip_prefix("--> ").map(str::trim));

        match site {
            Some(site) => errors.push(format!("{code} at {site}")),
            None => errors.push(code.to_string()),
        }
    }

    errors
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real stderr shape from the 2026-08-13 bastion compile break — `board.rs:660` matches the
    /// ticket's motivating incident (`MV.ticket.reconcile-failed-consumer`, 2 sites in bastion).
    const BASTION_BROKEN_STDERR: &str = "\
error[E0063]: missing field `blocked_by_kind` in initializer of `CarryoverEntry`
   --> src/serve/handlers/board.rs:660:9
    |
660 |         CarryoverEntry {
    |         ^^^^^^^^^^^^^^ missing `blocked_by_kind`

error[E0308]: mismatched types
   --> src/serve/handlers/block_graph.rs:414:22
    |
414 |     let kind: String = entry.blocked_by_kind;
    |                        ^^^^^^^^^^^^^^^^^^^^^ expected `String`, found `BlockedByKind`

error: aborting due to 2 previous errors

For more information about this error, try `rustc --explain E0063`.
error: could not compile `bastion` (lib test) due to 2 previous errors
";

    /// Real stderr shape from the 2026-08-13 engine-rs run — a stale lockfile (`sha2 0.10.9`),
    /// not a code break, surfaced under `--locked`.
    const ENGINE_RS_LOCKFILE_STALE_STDERR: &str = "\
error: failed to select a version for the requirement `sha2 = \"^0.10.9\"`
candidate versions found which didn't match: 0.10.8
location searched: crates.io index
required by package `engine-core v0.1.0 (/Users/brandon/Dev/agentic-portfolio/core/engine-rs/engine-core)`
if you are looking for the prior version of a crate to keep using, you can require an
older version by using `cargo update <pkg>@<current-ver> --precise <current-ver>`

Caused by:
  cannot update the lock file at Cargo.lock: --locked was passed to prevent this
";

    #[test]
    fn bastion_broken_stderr_classifies_as_broken_with_sites() {
        let outcome = classify(101, "", BASTION_BROKEN_STDERR, false);
        match outcome {
            ConsumerOutcome::Broken { errors } => {
                assert_eq!(errors.len(), 2);
                assert!(errors[0].contains("E0063"));
                assert!(errors[0].contains("board.rs:660"));
                assert!(errors[1].contains("E0308"));
                assert!(errors[1].contains("block_graph.rs:414"));
            }
            other => panic!("expected Broken, got {other:?}"),
        }
    }

    #[test]
    fn engine_rs_lockfile_stale_stderr_classifies_as_lockfile_stale() {
        let outcome = classify(102, "", ENGINE_RS_LOCKFILE_STALE_STDERR, false);
        assert_eq!(outcome, ConsumerOutcome::LockfileStale);
    }

    #[test]
    fn lockfile_stale_signature_wins_even_at_exit_101() {
        // The ticket's whole point: 101 was observed for BOTH a real break (bastion) and a
        // stale lock (a variant seen elsewhere) — the signature, not the exit code, decides.
        let outcome = classify(101, "", ENGINE_RS_LOCKFILE_STALE_STDERR, false);
        assert_eq!(outcome, ConsumerOutcome::LockfileStale);
    }

    #[test]
    fn exit_zero_is_pass() {
        assert_eq!(classify(0, "", "", false), ConsumerOutcome::Pass);
    }

    #[test]
    fn exit_zero_is_pass_even_with_noisy_stdout() {
        assert_eq!(
            classify(0, "test result: ok. 42 passed", "", false),
            ConsumerOutcome::Pass
        );
    }

    #[test]
    fn dirty_short_circuits_to_skipped_dirty_regardless_of_exit_code() {
        assert_eq!(
            classify(101, "", BASTION_BROKEN_STDERR, true),
            ConsumerOutcome::SkippedDirty
        );
        assert_eq!(classify(0, "", "", true), ConsumerOutcome::SkippedDirty);
        assert_eq!(
            classify(102, "", ENGINE_RS_LOCKFILE_STALE_STDERR, true),
            ConsumerOutcome::SkippedDirty
        );
    }

    #[test]
    fn unrecognised_failure_is_not_evaluable_with_reason_not_guessed_broken() {
        let outcome = classify(1, "", "signal: killed", false);
        match outcome {
            ConsumerOutcome::NotEvaluable { reason } => {
                assert!(reason.contains("1"));
                assert!(!reason.is_empty());
            }
            other => panic!("expected NotEvaluable, got {other:?}"),
        }
    }

    #[test]
    fn broken_error_without_site_line_falls_back_to_bare_code() {
        let stderr = "error[E0425]: cannot find value `x` in this scope\n";
        let outcome = classify(101, "", stderr, false);
        match outcome {
            ConsumerOutcome::Broken { errors } => {
                assert_eq!(errors, vec!["E0425".to_string()]);
            }
            other => panic!("expected Broken, got {other:?}"),
        }
    }

    #[test]
    fn consumer_result_carries_slug_and_outcome() {
        let result = ConsumerResult {
            slug: "bastion".to_string(),
            outcome: ConsumerOutcome::Pass,
        };
        assert_eq!(result.slug, "bastion");
        assert_eq!(result.outcome, ConsumerOutcome::Pass);
    }

    #[test]
    fn extract_compiler_errors_pure_no_io() {
        // Same function, called directly: proves it needs nothing but a string.
        let errors = extract_compiler_errors(BASTION_BROKEN_STDERR);
        assert_eq!(errors.len(), 2);
    }
}
