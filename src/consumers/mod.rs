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

use std::io;
use std::path::Path;
use std::process::Command;

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

/// Remove ANSI CSI escape sequences (`\x1b[...<letter>`) from `input`. rustc's colored output
/// wraps `error[E....]` in escapes like `\x1b[1m\x1b[38;5;9merror[E0308]\x1b[0m`, which defeats
/// a literal-prefix match on `"error["` even after `trim_start` (escapes are not whitespace).
/// Defense in depth alongside forcing `CARGO_TERM_COLOR=never` on the spawned command — this
/// keeps classification correct even if a future caller's environment forces color back on.
fn strip_ansi_codes(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next(); // consume '['
            for next in chars.by_ref() {
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Pull `error[E....]: ...` diagnostics out of rustc/cargo stderr, pairing each with its
/// `--> file:line:col` site when one appears in the following lines. Returns entries like
/// `"E0063 at src/serve/handlers/board.rs:660:9"`, or bare `"E0063"` if no site line was found.
fn extract_compiler_errors(stderr: &str) -> Vec<String> {
    let stripped = strip_ansi_codes(stderr);
    let lines: Vec<&str> = stripped.lines().collect();
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

// ---------------------------------------------------------------------------
// The single-consumer runner — does the I/O, hands off to `classify` for judgement.
// ---------------------------------------------------------------------------

/// The reduced result of spawning the compile-gate cargo invocation for one consumer — exactly
/// the inputs [`classify`] needs. Kept separate from [`std::process::Output`] so tests can
/// construct it directly instead of spawning a real process.
#[derive(Debug, Clone)]
pub struct SpawnOutcome {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Check `git -C <consumer_path> status --porcelain`. `Ok(true)` means the tree is dirty.
/// Errors (git unavailable, not a repo, etc.) are surfaced rather than silently treated as
/// clean — a consumer we cannot ask about is not evidence either way.
fn git_is_dirty(consumer_path: &Path) -> io::Result<bool> {
    let output = Command::new("git")
        .arg("-C")
        .arg(consumer_path)
        .args(["status", "--porcelain"])
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "git status --porcelain exited {:?} for {}",
            output.status.code(),
            consumer_path.display()
        )));
    }
    Ok(!output.stdout.is_empty())
}

/// Hash a consumer's `Cargo.lock` (`None` if it does not exist), for the before/after
/// byte-identity check that `--locked` is supposed to guarantee.
fn hash_lockfile(consumer_path: &Path) -> io::Result<Option<[u8; 32]>> {
    use sha2::{Digest, Sha256};
    let lock_path = consumer_path.join("Cargo.lock");
    if !lock_path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(&lock_path)?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(Some(hasher.finalize().into()))
}

/// First 8 hex chars of a lockfile hash, or `"missing"` — for a short, loud reason string, not
/// for cryptographic comparison (the `Option<[u8; 32]>` equality check does that).
fn digest_summary(hash: Option<[u8; 32]>) -> String {
    match hash {
        Some(bytes) => bytes.iter().take(4).map(|b| format!("{b:02x}")).collect(),
        None => "missing".to_string(),
    }
}

/// Spawn the real compile-gate command against `manifest_path`, in the fresh `target_dir` so it
/// never contends with that repo's own build lane. Every flag here is load-bearing (see the
/// module docs and the ticket) — do not simplify:
/// - `--no-run` compiles test targets without executing them; the break class this ticket
///   exists for lives only in test code, which `cargo build` cannot see.
/// - `--locked` refuses to rewrite a `Cargo.lock` we do not own, turning a silent mutation of
///   another repo into a loud error instead.
/// - a fresh `CARGO_TARGET_DIR` avoids `target/` lock contention with that consumer's own lane.
fn spawn_real(manifest_path: &Path, target_dir: &Path) -> io::Result<SpawnOutcome> {
    let output = Command::new("cargo")
        .env("CARGO_TARGET_DIR", target_dir)
        // Force plain output: on a CI runner that presents a pseudo-tty to spawned
        // subprocesses, rustc auto-detects color support and wraps `error[E....]` in ANSI
        // escapes, which `extract_compiler_errors`'s literal-prefix match cannot see through —
        // observed 2026-08-15 as a genuinely `Broken` consumer being classified
        // `NotEvaluable` in CI while passing locally. `extract_compiler_errors` also strips
        // ANSI as defense in depth, but forcing it off here is the real fix.
        .env("CARGO_TERM_COLOR", "never")
        .args(["nextest", "run", "--no-run", "--locked", "--manifest-path"])
        .arg(manifest_path)
        .output()?;
    Ok(SpawnOutcome {
        exit_code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

/// Run the compile gate against one consumer at `consumer_path`, identified by `slug`. Spawns
/// the real `cargo nextest run --no-run --locked` command — see [`run_consumer_with_spawner`]
/// for the testable, dependency-injected version.
pub fn run_consumer(slug: &str, consumer_path: &Path) -> ConsumerResult {
    run_consumer_with_spawner(slug, consumer_path, spawn_real)
}

/// Same as [`run_consumer`] but with the cargo spawn injected, so tests can assert whether it
/// was called at all — the dirty short-circuit's whole point — without spawning a real process.
///
/// Order of operations: check `git status --porcelain` FIRST and short-circuit to
/// `SkippedDirty` without ever calling `spawner` if the tree is dirty (a dirty consumer's
/// result is not evidence about mev's change either way); otherwise hash `Cargo.lock`, run the
/// spawner, re-hash `Cargo.lock`, and return `NotEvaluable` — never a guessed verdict — if the
/// hash moved, since that means `--locked` was dropped or defeated.
pub fn run_consumer_with_spawner<F>(slug: &str, consumer_path: &Path, spawner: F) -> ConsumerResult
where
    F: FnOnce(&Path, &Path) -> io::Result<SpawnOutcome>,
{
    let outcome = match git_is_dirty(consumer_path) {
        Ok(true) => ConsumerOutcome::SkippedDirty,
        Ok(false) => run_and_classify(consumer_path, spawner),
        Err(err) => ConsumerOutcome::NotEvaluable {
            reason: format!("could not determine git status for {slug}: {err}"),
        },
    };
    ConsumerResult {
        slug: slug.to_string(),
        outcome,
    }
}

fn run_and_classify<F>(consumer_path: &Path, spawner: F) -> ConsumerOutcome
where
    F: FnOnce(&Path, &Path) -> io::Result<SpawnOutcome>,
{
    let before = match hash_lockfile(consumer_path) {
        Ok(hash) => hash,
        Err(err) => {
            return ConsumerOutcome::NotEvaluable {
                reason: format!("could not read Cargo.lock before the run: {err}"),
            };
        }
    };

    let manifest_path = consumer_path.join("Cargo.toml");
    let target_dir = match tempfile::tempdir() {
        Ok(dir) => dir,
        Err(err) => {
            return ConsumerOutcome::NotEvaluable {
                reason: format!("could not create a fresh CARGO_TARGET_DIR: {err}"),
            };
        }
    };

    let spawned = match spawner(&manifest_path, target_dir.path()) {
        Ok(spawned) => spawned,
        Err(err) => {
            return ConsumerOutcome::NotEvaluable {
                reason: format!("could not spawn `cargo nextest run --no-run --locked`: {err}"),
            };
        }
    };

    let after = match hash_lockfile(consumer_path) {
        Ok(hash) => hash,
        Err(err) => {
            return ConsumerOutcome::NotEvaluable {
                reason: format!("could not read Cargo.lock after the run: {err}"),
            };
        }
    };

    if before != after {
        return ConsumerOutcome::NotEvaluable {
            reason: format!(
                "Cargo.lock changed during the run despite --locked ({} -> {}); this should \
                 never happen and means --locked was dropped or defeated",
                digest_summary(before),
                digest_summary(after),
            ),
        };
    }

    classify(spawned.exit_code, &spawned.stdout, &spawned.stderr, false)
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

    #[test]
    fn extract_compiler_errors_sees_through_ansi_color_codes() {
        // Reproduces the 2026-08-15 CI-only failure: a pseudo-tty makes rustc emit colored
        // diagnostics, wrapping "error[E0308]" in ANSI escapes that a literal-prefix match
        // cannot see through unless they are stripped first.
        let colored = "\u{1b}[0m\u{1b}[1m\u{1b}[38;5;9merror[E0308]\u{1b}[0m\u{1b}[1m: mismatched types\n \u{1b}[0m\u{1b}[0m\u{1b}[1m\u{1b}[38;5;12m--> \u{1b}[0msrc/lib.rs:5:23\n";
        let errors = extract_compiler_errors(colored);
        assert_eq!(errors, vec!["E0308 at src/lib.rs:5:23".to_string()]);
    }

    #[test]
    fn strip_ansi_codes_removes_csi_sequences_only() {
        assert_eq!(
            strip_ansi_codes("\u{1b}[1;31merror\u{1b}[0m[E0308]"),
            "error[E0308]"
        );
        assert_eq!(
            strip_ansi_codes("plain text, no escapes"),
            "plain text, no escapes"
        );
    }

    // -----------------------------------------------------------------
    // The runner: `run_consumer_with_spawner` — real git, injected cargo.
    // -----------------------------------------------------------------

    fn run_git(path: &std::path::Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .status()
            .expect("git must be on PATH for this test");
        assert!(status.success(), "git {args:?} failed in {path:?}");
    }

    /// A fresh, committed (clean) fixture consumer with a `Cargo.toml` and `Cargo.lock`.
    fn clean_fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        run_git(dir.path(), &["init"]);
        run_git(dir.path(), &["config", "user.email", "test@example.com"]);
        run_git(dir.path(), &["config", "user.name", "Test"]);
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\n",
        )
        .expect("write Cargo.toml");
        std::fs::write(dir.path().join("Cargo.lock"), "# fixture lockfile\n")
            .expect("write Cargo.lock");
        run_git(dir.path(), &["add", "-A"]);
        run_git(dir.path(), &["commit", "-m", "init"]);
        dir
    }

    #[test]
    fn dirty_consumer_short_circuits_without_spawning_cargo() {
        let dir = clean_fixture();
        // Make it dirty: an untracked file after the commit above.
        std::fs::write(dir.path().join("untracked.txt"), "oops").expect("write untracked");

        let spawned = std::rc::Rc::new(std::cell::Cell::new(false));
        let spawned_flag = spawned.clone();
        let result = run_consumer_with_spawner("fixture", dir.path(), move |_manifest, _target| {
            spawned_flag.set(true);
            Ok(SpawnOutcome {
                exit_code: 0,
                stdout: String::new(),
                stderr: String::new(),
            })
        });

        assert_eq!(result.slug, "fixture");
        assert_eq!(result.outcome, ConsumerOutcome::SkippedDirty);
        assert!(
            !spawned.get(),
            "cargo must never be spawned for a dirty consumer — skipping the spawn is the point"
        );
    }

    #[test]
    fn clean_consumer_runs_and_classifies_via_injected_spawner() {
        let dir = clean_fixture();

        let result = run_consumer_with_spawner("fixture", dir.path(), |_manifest, _target| {
            Ok(SpawnOutcome {
                exit_code: 0,
                stdout: "test result: ok".to_string(),
                stderr: String::new(),
            })
        });

        assert_eq!(result.slug, "fixture");
        assert_eq!(result.outcome, ConsumerOutcome::Pass);
    }

    #[test]
    fn clean_consumer_broken_stderr_classifies_as_broken() {
        let dir = clean_fixture();

        let result = run_consumer_with_spawner("fixture", dir.path(), |_manifest, _target| {
            Ok(SpawnOutcome {
                exit_code: 101,
                stdout: String::new(),
                stderr: BASTION_BROKEN_STDERR.to_string(),
            })
        });

        match result.outcome {
            ConsumerOutcome::Broken { errors } => assert_eq!(errors.len(), 2),
            other => panic!("expected Broken, got {other:?}"),
        }
    }

    #[test]
    fn lockfile_mutation_during_run_is_not_evaluable_not_broken() {
        let dir = clean_fixture();
        let consumer_path = dir.path().to_path_buf();
        let mutate_path = consumer_path.clone();

        let result =
            run_consumer_with_spawner("fixture", &consumer_path, move |_manifest, _target| {
                // Simulate `--locked` being defeated: mutate Cargo.lock as a side effect of
                // the "cargo run", exactly what the before/after hash check exists to catch.
                std::fs::write(mutate_path.join("Cargo.lock"), "# mutated\n")
                    .expect("write mutated Cargo.lock");
                Ok(SpawnOutcome {
                    exit_code: 0,
                    stdout: String::new(),
                    stderr: String::new(),
                })
            });

        match result.outcome {
            ConsumerOutcome::NotEvaluable { reason } => {
                assert!(
                    reason.contains("Cargo.lock changed"),
                    "reason should name the lockfile mutation, got: {reason}"
                );
            }
            other => panic!("expected NotEvaluable, got {other:?}"),
        }
    }

    #[test]
    fn run_consumer_wires_the_real_spawner() {
        // Not exercised against a real cargo project here (that belongs to task 4's live-fleet
        // verification) — just proves `run_consumer` composes with `spawn_real` and that a
        // dirty tree still short-circuits through the public, non-injected entry point.
        let dir = clean_fixture();
        std::fs::write(dir.path().join("untracked.txt"), "oops").expect("write untracked");

        let result = run_consumer("fixture", dir.path());

        assert_eq!(result.outcome, ConsumerOutcome::SkippedDirty);
    }
}
