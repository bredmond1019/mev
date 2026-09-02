//! Check `toolchain-freshness` — the running `mev` binary's build provenance vs the
//! source tree it was built from.
//!
//! This is the incident check: a `mev` binary built two days stale destroyed 29 authored
//! block notes in a single session because nothing compared it to its source before it
//! ran with `--write`. [`build.rs`](../../../build.rs) stamps three `cargo:rustc-env`
//! values into the binary at compile time (`MEV_BUILD_GIT_SHA`, `MEV_BUILD_DIRTY`,
//! `MEV_BUILD_SOURCE_DIR`); this check re-derives the live value now and compares.
//!
//! The verdict is a pure function of `(stamped_sha, live_sha, dirty, source_dir_exists)` —
//! [`verdict`] — so it is unit-tested directly without shelling out to git.

use std::path::Path;

use super::{CheckOutcome, CheckStatus, ConformanceCtx, FactSide};

const STAMPED_SHA: &str = env!("MEV_BUILD_GIT_SHA");
const STAMPED_DIRTY: &str = env!("MEV_BUILD_DIRTY");
const STAMPED_SOURCE_DIR: &str = env!("MEV_BUILD_SOURCE_DIR");

/// The pure verdict function: given the compiled-in stamp and the live state of the
/// source tree, decide pass / drift / not-evaluable. No I/O — callers gather the live
/// values (via git or otherwise) and pass them in, which is what makes this directly
/// unit-testable without shelling out to git in tests.
fn verdict(
    stamped_sha: &str,
    live_sha: Option<&str>,
    dirty: &str,
    source_dir_exists: bool,
) -> (CheckStatus, Vec<String>, Option<String>) {
    if stamped_sha == "unknown" || dirty == "unknown" || !source_dir_exists {
        return (
            CheckStatus::NotEvaluable,
            Vec::new(),
            Some(
                "build provenance unavailable: stamped SHA, dirty flag, or source dir missing"
                    .to_string(),
            ),
        );
    }

    let Some(live_sha) = live_sha else {
        return (
            CheckStatus::NotEvaluable,
            Vec::new(),
            Some(
                "could not determine the live HEAD of the source tree (git unavailable)"
                    .to_string(),
            ),
        );
    };

    if live_sha == "unknown" {
        return (
            CheckStatus::NotEvaluable,
            Vec::new(),
            Some(
                "could not determine the live HEAD of the source tree (git unavailable)"
                    .to_string(),
            ),
        );
    }

    if stamped_sha != live_sha {
        return (
            CheckStatus::Drift,
            vec![format!(
                "the running binary was built from {stamped_sha} but the source is now at \
                 {live_sha}; rebuild before any --write run"
            )],
            None,
        );
    }

    if dirty == "1" {
        return (
            CheckStatus::Drift,
            vec![
                "the binary was built from an uncommitted tree, so its provenance is \
                 unverifiable"
                    .to_string(),
            ],
            None,
        );
    }

    (CheckStatus::Pass, Vec::new(), None)
}

/// Build the `--build-stamp` JSON payload from raw stamp values, pure and testable without
/// touching the compiled-in consts.
///
/// This is a pinned cross-repo contract with bastion's `src/buildstamp.rs` (see
/// `MV.ticket.toolchain-freshness-covers-the-writer`) — the key set
/// (`git_sha`, `dirty`, `source_dir`) must never gain, lose, or rename a key. `dirty` is a
/// JSON boolean when the raw stamp is the literal `"0"`/`"1"`, and the JSON string
/// `"unknown"` for anything else — never guessed.
pub fn stamp_json_from(git_sha: &str, dirty: &str, source_dir: &str) -> serde_json::Value {
    let dirty_value = match dirty {
        "0" => serde_json::Value::Bool(false),
        "1" => serde_json::Value::Bool(true),
        _ => serde_json::Value::String("unknown".to_string()),
    };
    serde_json::json!({
        "git_sha": git_sha,
        "dirty": dirty_value,
        "source_dir": source_dir,
    })
}

/// The `--build-stamp` JSON payload for this compiled binary.
pub fn stamp_json() -> serde_json::Value {
    stamp_json_from(STAMPED_SHA, STAMPED_DIRTY, STAMPED_SOURCE_DIR)
}

/// Run `git rev-parse HEAD` in `source_dir` now, returning `None` if git or the command
/// is unavailable.
fn live_head(source_dir: &str) -> Option<String> {
    if !Path::new(source_dir).exists() {
        return None;
    }
    let output = crate::shared::git_command()
        .args(["rev-parse", "HEAD"])
        .current_dir(source_dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .map(|s| s.trim().to_string())
}

/// Other binaries in the fleet, beyond `mev` itself, that also run `--write` paths
/// against this corpus and must therefore be covered by `toolchain-freshness`.
///
/// The registry lives in `brain.toml`'s `[[conformance_writers]]` table (see
/// [`crate::brain::config::ConformanceWriter`]) — adding a writer is a config edit,
/// not a mev recompile.
use crate::brain::config::ConformanceWriter;

/// One registered writer's toolchain-freshness verdict, named, so a multi-writer report
/// can show a reader exactly which binary drifted or was not evaluable — never a bare
/// aggregate.
#[derive(Debug, Clone)]
pub struct WriterOutcome {
    pub name: String,
    pub status: CheckStatus,
    /// Drift details, already prefixed with `name`.
    pub findings: Vec<String>,
    /// Populated on `NotEvaluable`, already prefixed with `name`.
    pub reason: Option<String>,
}

fn status_label(status: CheckStatus) -> &'static str {
    match status {
        CheckStatus::Pass => "pass",
        CheckStatus::Drift => "drift",
        CheckStatus::NotEvaluable => "not_evaluable",
    }
}

/// Spawn `<name> --build-stamp` (resolved via `PATH`, or a literal path in tests) and
/// parse its stdout against the pinned `{git_sha, dirty, source_dir}` contract shared
/// with bastion's `src/buildstamp.rs`. Returns `Err` with a human-readable reason on any
/// failure — the binary isn't on `PATH`, it exited non-zero, or its stdout isn't the
/// expected JSON shape — so the caller reports `NotEvaluable` rather than ever silently
/// treating an unimplemented writer as `Pass`.
fn query_writer_stamp(name: &str) -> Result<(String, String, String), String> {
    let output = std::process::Command::new(name)
        .arg("--build-stamp")
        .output()
        .map_err(|e| format!("could not spawn `{name} --build-stamp`: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "`{name} --build-stamp` exited non-zero ({})",
            output.status
        ));
    }

    let stdout = String::from_utf8(output.stdout)
        .map_err(|e| format!("`{name} --build-stamp` stdout was not valid UTF-8: {e}"))?;

    let value: serde_json::Value = serde_json::from_str(stdout.trim())
        .map_err(|e| format!("`{name} --build-stamp` stdout was not valid JSON: {e}"))?;

    let git_sha = value
        .get("git_sha")
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("`{name} --build-stamp` JSON missing string `git_sha`"))?
        .to_string();

    let source_dir = value
        .get("source_dir")
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("`{name} --build-stamp` JSON missing string `source_dir`"))?
        .to_string();

    let dirty = match value.get("dirty") {
        Some(serde_json::Value::Bool(true)) => "1".to_string(),
        Some(serde_json::Value::Bool(false)) => "0".to_string(),
        Some(serde_json::Value::String(s)) if s == "unknown" => "unknown".to_string(),
        _ => {
            return Err(format!(
                "`{name} --build-stamp` JSON `dirty` was not a boolean or \"unknown\""
            ));
        }
    };

    Ok((git_sha, dirty, source_dir))
}

/// Run the pure [`verdict`] for one already-resolved `(stamped_sha, dirty, source_dir)`
/// triple, naming `name` in every finding/reason.
fn writer_outcome(name: &str, stamped_sha: &str, dirty: &str, source_dir: &str) -> WriterOutcome {
    let source_dir_exists = Path::new(source_dir).exists();
    let live_sha = if source_dir_exists {
        live_head(source_dir)
    } else {
        None
    };
    let (status, findings, reason) =
        verdict(stamped_sha, live_sha.as_deref(), dirty, source_dir_exists);
    WriterOutcome {
        name: name.to_string(),
        status,
        findings: findings
            .into_iter()
            .map(|f| format!("{name}: {f}"))
            .collect(),
        reason: reason.map(|r| format!("{name}: {r}")),
    }
}

/// Query and evaluate one cross-binary writer named in the `[[conformance_writers]]`
/// registry. A spawn failure, non-zero exit, or unparseable/wrong-shaped stdout all
/// produce `NotEvaluable` named for `writer.name` — never `Pass` and never silently
/// skipped. When `repo_path` is set, it is appended to the `NotEvaluable` reason so a
/// reader can find the source repo of a writer that could not be queried.
fn cross_binary_outcome(writer: &ConformanceWriter) -> WriterOutcome {
    let name = writer.name.as_str();
    match query_writer_stamp(name) {
        Ok((git_sha, dirty, source_dir)) => writer_outcome(name, &git_sha, &dirty, &source_dir),
        Err(reason) => {
            let reason = match &writer.repo_path {
                Some(repo_path) => format!("{name}: {reason} (repo_path: {repo_path})"),
                None => format!("{name}: {reason}"),
            };
            WriterOutcome {
                name: name.to_string(),
                status: CheckStatus::NotEvaluable,
                findings: Vec::new(),
                reason: Some(reason),
            }
        }
    }
}

/// Worst-wins aggregation across every registered writer: `Drift` beats
/// `NotEvaluable` beats `Pass`.
fn worst_status(a: CheckStatus, b: CheckStatus) -> CheckStatus {
    match (a, b) {
        (CheckStatus::Drift, _) | (_, CheckStatus::Drift) => CheckStatus::Drift,
        (CheckStatus::NotEvaluable, _) | (_, CheckStatus::NotEvaluable) => {
            CheckStatus::NotEvaluable
        }
        _ => CheckStatus::Pass,
    }
}

/// Compute the per-writer outcomes (`self` + every entry in `writers`) alongside the
/// worst-wins overall status. An empty registry degrades to mev-only: exactly one
/// outcome, named `self`, never a panic or a silent pass.
pub fn writer_outcomes(writers: &[ConformanceWriter]) -> (CheckStatus, Vec<WriterOutcome>) {
    let mut outcomes = vec![writer_outcome(
        "self",
        STAMPED_SHA,
        STAMPED_DIRTY,
        STAMPED_SOURCE_DIR,
    )];
    for writer in writers {
        outcomes.push(cross_binary_outcome(writer));
    }
    let mut overall_status = CheckStatus::Pass;
    for outcome in &outcomes {
        overall_status = worst_status(overall_status, outcome.status);
    }
    (overall_status, outcomes)
}

/// Run the `toolchain-freshness` check across every registered writer: `mev` itself
/// (the compiled-in stamp, as before) plus every writer named in `brain.toml`'s
/// `[[conformance_writers]]` table (`ctx.config.conformance_writers`), queried via
/// `--build-stamp`. The overall status is worst-wins; `findings` names every writer's
/// individual verdict so a reader can see exactly which binary drifted or could not be
/// evaluated, not just an aggregate.
pub fn run(ctx: &ConformanceCtx) -> CheckOutcome {
    let (_, outcomes) = writer_outcomes(&ctx.config.conformance_writers);

    let left = FactSide {
        label: "compiled-in build stamp (self)".to_string(),
        source: "MEV_BUILD_GIT_SHA / MEV_BUILD_DIRTY (env! at compile time)".to_string(),
        digest: super::digest(&[STAMPED_SHA.to_string()]),
        items: vec![
            format!("self: sha={STAMPED_SHA}"),
            format!("self: dirty={STAMPED_DIRTY}"),
        ],
    };

    let mut overall_status = CheckStatus::Pass;
    let mut findings = Vec::new();
    let mut reasons = Vec::new();
    let mut right_items = Vec::new();

    for outcome in &outcomes {
        overall_status = worst_status(overall_status, outcome.status);
        findings.extend(outcome.findings.clone());
        if let Some(reason) = &outcome.reason {
            reasons.push(reason.clone());
        }
        right_items.push(format!(
            "{}: {}",
            outcome.name,
            status_label(outcome.status)
        ));
    }

    let right = FactSide {
        label: "per-writer verdict (self + brain.toml [[conformance_writers]])".to_string(),
        source: "self's live source tree HEAD; each cross-binary writer via `--build-stamp`"
            .to_string(),
        digest: super::digest(&right_items),
        items: right_items,
    };

    let reason = if reasons.is_empty() {
        None
    } else {
        Some(reasons.join("; "))
    };

    CheckOutcome {
        status: overall_status,
        left: Some(left),
        right: Some(right),
        findings,
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_evaluable_when_stamped_sha_unknown() {
        let (status, _findings, reason) = verdict("unknown", Some("abc123"), "0", true);
        assert_eq!(status, CheckStatus::NotEvaluable);
        assert!(reason.is_some());
    }

    #[test]
    fn not_evaluable_when_dirty_flag_unknown() {
        let (status, _findings, reason) = verdict("abc123", Some("abc123"), "unknown", true);
        assert_eq!(status, CheckStatus::NotEvaluable);
        assert!(reason.is_some());
    }

    #[test]
    fn not_evaluable_when_source_dir_missing() {
        let (status, _findings, reason) = verdict("abc123", Some("abc123"), "0", false);
        assert_eq!(status, CheckStatus::NotEvaluable);
        assert!(reason.is_some());
    }

    #[test]
    fn not_evaluable_when_live_sha_unavailable() {
        let (status, _findings, reason) = verdict("abc123", None, "0", true);
        assert_eq!(status, CheckStatus::NotEvaluable);
        assert!(reason.is_some());
    }

    #[test]
    fn not_evaluable_when_live_sha_literal_unknown() {
        let (status, _findings, reason) = verdict("abc123", Some("unknown"), "0", true);
        assert_eq!(status, CheckStatus::NotEvaluable);
        assert!(reason.is_some());
    }

    #[test]
    fn drift_when_sha_differs() {
        let (status, findings, reason) = verdict("abc123", Some("def456"), "0", true);
        assert_eq!(status, CheckStatus::Drift);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].contains("abc123"));
        assert!(findings[0].contains("def456"));
        assert!(findings[0].contains("rebuild"));
        assert!(reason.is_none());
    }

    #[test]
    fn drift_when_dirty_even_with_matching_sha() {
        let (status, findings, reason) = verdict("abc123", Some("abc123"), "1", true);
        assert_eq!(status, CheckStatus::Drift);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].contains("uncommitted"));
        assert!(reason.is_none());
    }

    #[test]
    fn dirty_drift_message_distinct_from_stale_sha_drift_message() {
        let (_status1, stale_findings, _) = verdict("abc123", Some("def456"), "0", true);
        let (_status2, dirty_findings, _) = verdict("abc123", Some("abc123"), "1", true);
        assert_ne!(stale_findings[0], dirty_findings[0]);
    }

    #[test]
    fn pass_when_sha_matches_and_clean() {
        let (status, findings, reason) = verdict("abc123", Some("abc123"), "0", true);
        assert_eq!(status, CheckStatus::Pass);
        assert!(findings.is_empty());
        assert!(reason.is_none());
    }

    #[test]
    fn stamp_json_from_reports_clean_boolean_dirty() {
        let v = stamp_json_from("abc123", "0", "/tmp/src");
        assert_eq!(v["git_sha"], serde_json::json!("abc123"));
        assert_eq!(v["dirty"], serde_json::json!(false));
        assert_eq!(v["source_dir"], serde_json::json!("/tmp/src"));
    }

    #[test]
    fn stamp_json_from_reports_dirty_boolean_true() {
        let v = stamp_json_from("abc123", "1", "/tmp/src");
        assert_eq!(v["dirty"], serde_json::json!(true));
    }

    #[test]
    fn stamp_json_from_reports_unknown_dirty_as_string_not_guessed() {
        let v = stamp_json_from("unknown", "unknown", "/tmp/src");
        assert_eq!(v["dirty"], serde_json::json!("unknown"));
    }

    #[test]
    fn stamp_json_from_has_exactly_three_keys() {
        let v = stamp_json_from("abc123", "0", "/tmp/src");
        let obj = v.as_object().expect("stamp json must be an object");
        assert_eq!(obj.len(), 3);
        assert!(obj.contains_key("git_sha"));
        assert!(obj.contains_key("dirty"));
        assert!(obj.contains_key("source_dir"));
    }

    #[test]
    fn stamp_json_uses_compiled_in_consts() {
        let v = stamp_json();
        assert_eq!(v["git_sha"], serde_json::json!(STAMPED_SHA));
        assert_eq!(v["source_dir"], serde_json::json!(STAMPED_SOURCE_DIR));
    }

    #[test]
    fn worst_status_drift_beats_everything() {
        assert_eq!(
            worst_status(CheckStatus::Pass, CheckStatus::Drift),
            CheckStatus::Drift
        );
        assert_eq!(
            worst_status(CheckStatus::NotEvaluable, CheckStatus::Drift),
            CheckStatus::Drift
        );
        assert_eq!(
            worst_status(CheckStatus::Drift, CheckStatus::Pass),
            CheckStatus::Drift
        );
    }

    #[test]
    fn worst_status_not_evaluable_beats_pass() {
        assert_eq!(
            worst_status(CheckStatus::Pass, CheckStatus::NotEvaluable),
            CheckStatus::NotEvaluable
        );
        assert_eq!(
            worst_status(CheckStatus::NotEvaluable, CheckStatus::Pass),
            CheckStatus::NotEvaluable
        );
    }

    #[test]
    fn worst_status_pass_when_both_pass() {
        assert_eq!(
            worst_status(CheckStatus::Pass, CheckStatus::Pass),
            CheckStatus::Pass
        );
    }

    /// Write an executable shell script at a temp path that prints `stdout` and exits
    /// `exit_code`, returning the script's absolute path. `query_writer_stamp` accepts a
    /// literal path (it's just `Command::new(name)`), so tests don't need to touch `PATH`.
    fn fake_writer_script(test_name: &str, stdout: &str, exit_code: i32) -> std::path::PathBuf {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;

        let mut path = std::env::temp_dir();
        path.push(format!(
            "mev_toolchain_test_{test_name}_{}",
            std::process::id()
        ));
        let mut file = std::fs::File::create(&path).expect("create fake writer script");
        writeln!(file, "#!/bin/sh").expect("write shebang");
        writeln!(file, "cat <<'EOF'\n{stdout}\nEOF").expect("write body");
        writeln!(file, "exit {exit_code}").expect("write exit");
        drop(file);
        let mut perms = std::fs::metadata(&path)
            .expect("stat fake writer script")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).expect("chmod fake writer script");
        path
    }

    #[test]
    fn query_writer_stamp_parses_valid_json() {
        let script = fake_writer_script(
            "valid",
            r#"{"git_sha":"abc123","dirty":false,"source_dir":"/tmp/src"}"#,
            0,
        );
        let result = query_writer_stamp(script.to_str().unwrap());
        let _ = std::fs::remove_file(&script);
        let (git_sha, dirty, source_dir) = result.expect("valid stamp should parse");
        assert_eq!(git_sha, "abc123");
        assert_eq!(dirty, "0");
        assert_eq!(source_dir, "/tmp/src");
    }

    #[test]
    fn query_writer_stamp_rejects_non_zero_exit() {
        let script = fake_writer_script("nonzero", r#"{"git_sha":"abc"}"#, 1);
        let result = query_writer_stamp(script.to_str().unwrap());
        let _ = std::fs::remove_file(&script);
        assert!(result.is_err());
    }

    #[test]
    fn query_writer_stamp_rejects_unparseable_json() {
        let script = fake_writer_script("badjson", "not json at all", 0);
        let result = query_writer_stamp(script.to_str().unwrap());
        let _ = std::fs::remove_file(&script);
        assert!(result.is_err());
    }

    #[test]
    fn query_writer_stamp_rejects_missing_binary() {
        let result = query_writer_stamp("/definitely/not/a/real/path/mev_test_missing_binary");
        assert!(result.is_err());
    }

    #[test]
    fn query_writer_stamp_rejects_bad_dirty_shape() {
        let script = fake_writer_script(
            "baddirty",
            r#"{"git_sha":"abc123","dirty":"not-a-bool","source_dir":"/tmp/src"}"#,
            0,
        );
        let result = query_writer_stamp(script.to_str().unwrap());
        let _ = std::fs::remove_file(&script);
        assert!(result.is_err());
    }

    #[test]
    fn cross_binary_outcome_names_missing_writer_not_evaluable_never_pass() {
        let writer = ConformanceWriter {
            name: "/definitely/not/a/real/path/mev_test_ghost_writer".to_string(),
            repo_path: None,
        };
        let outcome = cross_binary_outcome(&writer);
        assert_eq!(outcome.status, CheckStatus::NotEvaluable);
        assert_ne!(outcome.status, CheckStatus::Pass);
        assert!(outcome.reason.is_some());
        assert!(
            outcome
                .reason
                .as_ref()
                .unwrap()
                .contains("mev_test_ghost_writer")
        );
    }

    #[test]
    fn cross_binary_outcome_not_evaluable_reason_names_repo_path() {
        let writer = ConformanceWriter {
            name: "/definitely/not/a/real/path/mev_test_ghost_writer".to_string(),
            repo_path: Some("bastion".to_string()),
        };
        let outcome = cross_binary_outcome(&writer);
        assert_eq!(outcome.status, CheckStatus::NotEvaluable);
        assert!(outcome.reason.as_ref().unwrap().contains("bastion"));
    }

    #[test]
    fn writer_outcome_drift_names_the_binary() {
        // A writer claiming a source_dir that exists (".") but a sha that cannot match
        // live HEAD in that dir yields Drift, named for that writer.
        let outcome = writer_outcome("bastion", "not-a-real-sha-ever", "0", ".");
        // Only assert Drift when live_head could actually be resolved (git present);
        // otherwise this legitimately falls back to NotEvaluable, which is also a valid,
        // named (never-Pass) outcome.
        assert_ne!(outcome.status, CheckStatus::Pass);
        match outcome.status {
            CheckStatus::Drift => {
                assert!(outcome.findings.iter().any(|f| f.starts_with("bastion:")));
            }
            CheckStatus::NotEvaluable => {
                assert!(
                    outcome
                        .reason
                        .as_ref()
                        .is_some_and(|r| r.starts_with("bastion:"))
                );
            }
            CheckStatus::Pass => unreachable!(),
        }
    }

    #[test]
    fn run_aggregates_worst_across_writers_and_names_each() {
        let mut config = crate::brain::config::BrainConfig::default();
        config.conformance_writers = vec![ConformanceWriter {
            name: "bastion".to_string(),
            repo_path: Some("bastion".to_string()),
        }];
        let ctx = ConformanceCtx {
            root: std::path::PathBuf::from("."),
            config,
            files: Vec::new(),
        };
        let outcome = run(&ctx);
        // self is always represented in the report.
        assert!(
            outcome
                .right
                .as_ref()
                .unwrap()
                .items
                .iter()
                .any(|i| i.starts_with("self:"))
        );
        // every registered writer is named too, whatever its verdict.
        for writer in &ctx.config.conformance_writers {
            assert!(
                outcome
                    .right
                    .as_ref()
                    .unwrap()
                    .items
                    .iter()
                    .any(|i| i.starts_with(&format!("{}:", writer.name)))
            );
        }
    }

    #[test]
    fn run_executes_without_panicking() {
        // Smoke test: the real `run` function reads the actual compiled-in stamp and
        // shells out to the real source dir. It must not panic regardless of the
        // environment this test runs in (git present or not, source dir intact or not).
        let ctx = ConformanceCtx {
            root: std::path::PathBuf::from("."),
            config: crate::brain::config::BrainConfig::default(),
            files: Vec::new(),
        };
        let outcome = run(&ctx);
        assert!(outcome.left.is_some());
        assert!(outcome.right.is_some());
    }

    #[test]
    fn writer_outcomes_empty_registry_degrades_to_self_only() {
        // An empty `[[conformance_writers]]` registry must not panic or silently pass
        // extra writers — exactly one outcome, named `self`.
        let (status, outcomes) = writer_outcomes(&[]);
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].name, "self");
        // Status is whatever self's compiled-in stamp evaluates to (Pass or
        // NotEvaluable depending on the build environment), never a panic.
        let _ = status;
    }
}
