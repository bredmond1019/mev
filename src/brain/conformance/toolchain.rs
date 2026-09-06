//! Check `toolchain-freshness` — the running `mev` binary's build provenance vs the
//! source tree it was built from.
//!
//! This is the incident check: a `mev` binary built two days stale destroyed 29 authored
//! block notes in a single session because nothing compared it to its source before it
//! ran with `--write`. [`build.rs`](../../../build.rs) stamps three `cargo:rustc-env`
//! values into the binary at compile time (`MEV_BUILD_GIT_SHA`, `MEV_BUILD_DIRTY`,
//! `MEV_BUILD_SOURCE_DIR`); this check re-derives the live value now and compares.
//!
//! The verdict is a pure function of `(stamped_sha, live_sha, dirty, source_dir_exists,
//! build_inputs)` — [`verdict`] — so it is unit-tested directly without shelling out to
//! git. When the SHAs differ, a Drift is reported only if the build inputs actually
//! changed between the two commits (or that comparison could not be made); a non-build
//! difference — a docs edit, a `log.md` line, a harness sync — reports Pass instead.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::{CheckOutcome, CheckStatus, ConformanceCtx, FactSide};

const STAMPED_SHA: &str = env!("MEV_BUILD_GIT_SHA");
const STAMPED_DIRTY: &str = env!("MEV_BUILD_DIRTY");
const STAMPED_SOURCE_DIR: &str = env!("MEV_BUILD_SOURCE_DIR");

/// Build input paths for this repo: anything that can change the compiled `mev` binary.
/// `Cargo.lock` is included deliberately — a dependency bump changes the binary with no
/// first-party source change. `crates/` matters as of the learn-ai extraction: a
/// workspace member is a build input too.
const BUILD_INPUT_PATHS: &[&str] = &[
    "src/",
    "crates/",
    "tests/",
    "build.rs",
    "Cargo.toml",
    "Cargo.lock",
    ".cargo/",
];

/// Whether the build inputs differ between two commits — the answer to "does the source
/// change between `stamped_sha` and `live_sha` actually touch anything that affects the
/// compiled binary?" `Unknown` is not optional and not cosmetic: it is what a caller
/// reports when git cannot answer (e.g. the stamped SHA was rebased or garbage-collected
/// away), and absence of a diff answer must never be read as "no difference" — callers
/// treat `Unknown` the same as `Differ`.
///
/// `pub` (rather than `pub(crate)`) and `#[doc(hidden)]` only so `tests/it` — a separate
/// integration-test crate — can assert on [`differ_build_inputs`] directly, matching the
/// `testsupport` module's precedent for exposing impure test-only surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[doc(hidden)]
pub enum BuildInputComparison {
    /// `git diff` between the two commits over the build input paths reported no change.
    Same,
    /// `git diff` between the two commits over the build input paths reported a change.
    Differ,
    /// The comparison could not be made at all (unresolvable SHA, git unavailable, etc).
    Unknown,
}

/// Whether a writer's Cargo path-dependency closure (see [`path_dependency_closure`]) has
/// build-input commits newer than the writer's own binary build time. This is a SEPARATE
/// signal from [`BuildInputComparison`] — that one compares the writer's OWN repo between
/// two commits; this one asks whether a SIBLING repo the writer path-depends on moved
/// since the writer was actually compiled, which a same-repo `git diff` can never see.
///
/// Follows `BuildInputComparison`'s doctrine verbatim: `Unknown` is not cosmetic. Absence
/// of an answer (the writer's build time or a dependency's commit history could not be
/// determined) must never be read as "no difference" — callers treat `Unknown` the same
/// as `Differ`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[doc(hidden)]
pub enum PathDepComparison {
    /// The writer's manifest has no path dependencies at all (repo-local or transitive).
    NoPathDeps,
    /// Every path dependency's build inputs are unchanged since the writer's build time.
    Same,
    /// At least one path dependency has a build-input commit newer than the writer's
    /// build time. Carries the dependency directory names (e.g. `okf-core`) so the
    /// finding can name exactly which sibling repo moved.
    Differ(Vec<String>),
    /// The comparison could not be made (writer executable not found/stat-able, or a
    /// dependency's commit history could not be read).
    Unknown,
}

/// The pure verdict function: given the compiled-in stamp, the live state of the source
/// tree, (when the SHAs differ) an already-computed answer to whether the build inputs
/// actually changed between the two commits, and an already-computed path-dependency
/// freshness comparison, decide pass / drift / not-evaluable. No I/O — callers gather the
/// live values (via git or otherwise) and pass them in, which is what makes this directly
/// unit-testable without shelling out to git in tests.
fn verdict(
    stamped_sha: &str,
    live_sha: Option<&str>,
    dirty: &str,
    source_dir_exists: bool,
    build_inputs: BuildInputComparison,
    path_deps: PathDepComparison,
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

    // `dirty` is checked BEFORE the SHA-differs branch below, deliberately: a binary
    // built from an uncommitted tree has unverifiable provenance regardless of what the
    // build-input comparison says, and it must win even when `build_inputs` independently
    // reports `Same` for the (stamped_sha, live_sha) pair. Checking it only after a SHA
    // mismatch previously let a `Same` verdict return Pass early and skip this check
    // entirely whenever the SHAs also happened to differ — the collapse this ticket's
    // fixtures exist to catch.
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

    if stamped_sha != live_sha {
        match build_inputs {
            BuildInputComparison::Same => {
                // A path-dependency drift is an ADDITIONAL way to reach Drift here, never
                // a way to turn an existing Drift into a Pass — this branch only runs
                // when the writer's OWN repo comparison already says `Same`.
                if let Some(drift) = path_dep_drift(&path_deps) {
                    return drift;
                }
                return (
                    CheckStatus::Pass,
                    vec![format!(
                        "the running binary was built from {stamped_sha} and the source is now \
                         at {live_sha}, but nothing in that difference touches a build input \
                         (src/, crates/, tests/, build.rs, Cargo.toml, Cargo.lock, .cargo/) — \
                         the binary is still current"
                    )],
                    None,
                );
            }
            BuildInputComparison::Differ => {
                return (
                    CheckStatus::Drift,
                    vec![format!(
                        "the running binary was built from {stamped_sha} but the source is now \
                         at {live_sha}; rebuild before any --write run"
                    )],
                    None,
                );
            }
            BuildInputComparison::Unknown => {
                return (
                    CheckStatus::Drift,
                    vec![format!(
                        "the running binary was built from {stamped_sha} but the source is now \
                         at {live_sha}; rebuild before any --write run (build-input comparison \
                         could not be made between these two commits, so this is treated as \
                         drift)"
                    )],
                    None,
                );
            }
        }
    }

    // SHAs match (or `live_sha` was unavailable to compare against — already handled
    // above): the writer's own repo is current. A path dependency can still make it
    // stale, which the same-repo SHA comparison above can never see.
    if let Some(drift) = path_dep_drift(&path_deps) {
        return drift;
    }

    (CheckStatus::Pass, Vec::new(), None)
}

/// Turn a [`PathDepComparison`] into a Drift verdict, or `None` when it does not warrant
/// one. `NoPathDeps` and `Same` both mean "nothing to add" — the caller falls through to
/// whatever Pass verdict it already had. `Differ` names the dependency repos that moved;
/// `Unknown` is treated exactly the same as `Differ`, per this module's doctrine that an
/// unanswerable comparison must never read as "no difference".
fn path_dep_drift(
    path_deps: &PathDepComparison,
) -> Option<(CheckStatus, Vec<String>, Option<String>)> {
    match path_deps {
        PathDepComparison::NoPathDeps | PathDepComparison::Same => None,
        PathDepComparison::Differ(names) => Some((
            CheckStatus::Drift,
            vec![format!(
                "a path dependency has build-input commits newer than this binary's build \
                 time ({}); rebuild before any --write run",
                names.join(", ")
            )],
            None,
        )),
        PathDepComparison::Unknown => Some((
            CheckStatus::Drift,
            vec![
                "a path dependency's freshness could not be determined (build time or \
                 dependency history unavailable); treated as drift"
                    .to_string(),
            ],
            None,
        )),
    }
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

/// Ask git whether the build inputs (`BUILD_INPUT_PATHS`) differ between two commits in
/// `source_dir`. Modelled on `live_head`: a thin, impure wrapper around a single git
/// invocation, kept separate from the pure `verdict` so the decision logic stays
/// unit-testable without shelling out.
///
/// Runs `git diff --quiet <from> <to> -- <build input paths>`. Exit 0 -> `Same`, exit 1
/// -> `Differ`, anything else — including an unresolvable SHA, git being unavailable, or
/// the process failing to spawn — -> `Unknown`. `Unknown` is never treated as `Same`; the
/// caller (`verdict`) reports it as Drift.
///
/// `pub`/`#[doc(hidden)]` for the same reason as [`BuildInputComparison`]: `tests/it`
/// needs to exercise the real git-shelling comparison against a throwaway fixture repo,
/// and a separate integration-test crate can only link `pub` items.
#[doc(hidden)]
pub fn differ_build_inputs(source_dir: &str, from: &str, to: &str) -> BuildInputComparison {
    let mut args: Vec<&str> = vec!["diff", "--quiet", from, to, "--"];
    args.extend(BUILD_INPUT_PATHS.iter().copied());
    let output = crate::shared::git_command()
        .args(&args)
        .current_dir(source_dir)
        .output();
    let Ok(output) = output else {
        return BuildInputComparison::Unknown;
    };
    match output.status.code() {
        Some(0) => BuildInputComparison::Same,
        Some(1) => BuildInputComparison::Differ,
        _ => BuildInputComparison::Unknown,
    }
}

/// Parse `<source_dir>/Cargo.toml` for `path = "..."` dependency entries (across
/// `[dependencies]`, `[dev-dependencies]`, `[build-dependencies]`, and their
/// target-specific `[target.'cfg(...)'.*]` equivalents), resolve each relative to the
/// manifest's own directory, and recurse TRANSITIVELY with a visited-set so a dependency
/// cycle (`a -> b -> a`) cannot hang the check.
///
/// Returns the resolved, canonicalized directories of every path dependency reachable
/// from `source_dir`'s manifest — `source_dir` itself is never included. A missing or
/// unparseable manifest, or a path entry that does not resolve to a real directory, is
/// simply skipped rather than treated as an error: the caller reads an empty closure as
/// `PathDepComparison::NoPathDeps`, never a hard failure.
///
/// `pub`/`#[doc(hidden)]` for the same reason as [`BuildInputComparison`] and
/// [`differ_build_inputs`]: `tests/it` needs to exercise the real manifest-walking logic
/// against throwaway fixture repos.
#[doc(hidden)]
pub fn path_dependency_closure(source_dir: &str) -> Vec<PathBuf> {
    let mut visited: HashSet<PathBuf> = HashSet::new();
    let mut result = Vec::new();
    let Ok(start) = Path::new(source_dir).canonicalize() else {
        return result;
    };
    visited.insert(start.clone());
    collect_path_dependencies(&start, &mut visited, &mut result);
    result
}

/// Recursive worker for [`path_dependency_closure`]: reads one manifest directory's
/// `path = "..."` entries, appends any not-yet-visited resolved directory to `result`,
/// marks it visited, and recurses into it. The visited-set (keyed by canonicalized path)
/// is what makes a dependency cycle terminate instead of hang.
fn collect_path_dependencies(
    manifest_dir: &Path,
    visited: &mut HashSet<PathBuf>,
    result: &mut Vec<PathBuf>,
) {
    let manifest_path = manifest_dir.join("Cargo.toml");
    let Ok(contents) = std::fs::read_to_string(&manifest_path) else {
        return;
    };
    let Ok(value) = contents.parse::<toml::Value>() else {
        return;
    };

    for table_name in ["dependencies", "dev-dependencies", "build-dependencies"] {
        collect_from_dependency_table(value.get(table_name), manifest_dir, visited, result);
    }

    // Target-specific dependency tables: `[target.'cfg(...)'.dependencies]` and friends.
    if let Some(target_table) = value.get("target").and_then(|t| t.as_table()) {
        for cfg_table in target_table.values() {
            for table_name in ["dependencies", "dev-dependencies", "build-dependencies"] {
                collect_from_dependency_table(
                    cfg_table.get(table_name),
                    manifest_dir,
                    visited,
                    result,
                );
            }
        }
    }
}

/// Walk one `[dependencies]`-shaped TOML table, resolving every `path = "..."` entry
/// relative to `manifest_dir`, appending unvisited ones to `result`/`visited`, and
/// recursing into each.
fn collect_from_dependency_table(
    table: Option<&toml::Value>,
    manifest_dir: &Path,
    visited: &mut HashSet<PathBuf>,
    result: &mut Vec<PathBuf>,
) {
    let Some(table) = table.and_then(|t| t.as_table()) else {
        return;
    };
    for spec in table.values() {
        let Some(path_str) = spec.get("path").and_then(|p| p.as_str()) else {
            continue;
        };
        let Ok(dep_dir) = manifest_dir.join(path_str).canonicalize() else {
            continue;
        };
        if !visited.insert(dep_dir.clone()) {
            continue;
        }
        result.push(dep_dir.clone());
        collect_path_dependencies(&dep_dir, visited, result);
    }
}

/// Latest commit time touching `BUILD_INPUT_PATHS` inside `dep_dir`, via
/// `git log -1 --format=%ct -- <build input paths>`. `None` means the comparison could
/// not be made at all (git unavailable, `dep_dir` isn't a repo, or the timestamp couldn't
/// be parsed) — the caller treats that as [`PathDepComparison::Unknown`], never as "not
/// newer". An empty result (git ran fine but no commit has ever touched a build input in
/// this dir) is answered as the Unix epoch: a real, very-old answer, not an unknown one.
fn last_build_input_commit_time(dep_dir: &Path) -> Option<SystemTime> {
    let mut args: Vec<&str> = vec!["log", "-1", "--format=%ct", "--"];
    args.extend(BUILD_INPUT_PATHS.iter().copied());
    let output = crate::shared::git_command()
        .args(&args)
        .current_dir(dep_dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Some(SystemTime::UNIX_EPOCH);
    }
    let secs: u64 = trimmed.parse().ok()?;
    Some(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(secs))
}

/// Resolve the executable backing a writer `name` to a concrete path so its mtime can be
/// stat'd. A `name` containing a path separator (or absolute) is used literally — matching
/// how tests and [`query_writer_stamp`] already pass full paths; otherwise `PATH` is
/// searched by hand, mirroring how [`std::process::Command`] would have found it.
fn resolve_executable_path(name: &str) -> Option<PathBuf> {
    let candidate = Path::new(name);
    if candidate.components().count() > 1 {
        return candidate.is_file().then(|| candidate.to_path_buf());
    }
    let path_var = std::env::var_os("PATH")?;
    std::env::split_paths(&path_var)
        .map(|dir| dir.join(name))
        .find(|full| full.is_file())
}

/// The build time of writer `name`'s compiled binary: the executable's own mtime. For
/// `self` (the currently-running `mev` process) that is
/// [`std::env::current_exe`]; for any other registered writer it is the executable
/// resolved the same way the registry already invokes it — by name on `PATH` (or a
/// literal path in tests). `None` if the executable cannot be found or stat'd, which the
/// caller reads as [`PathDepComparison::Unknown`].
fn writer_build_time(name: &str) -> Option<SystemTime> {
    let exe_path = if name == "self" {
        std::env::current_exe().ok()?
    } else {
        resolve_executable_path(name)?
    };
    std::fs::metadata(&exe_path).ok()?.modified().ok()
}

/// Compute [`PathDepComparison`] for a writer built at `build_time` whose source lives at
/// `source_dir`: walk its transitive Cargo path-dependency closure, and for each
/// dependency ask whether its own build inputs have a commit newer than `build_time`. A
/// single unresolvable dependency makes the WHOLE comparison `Unknown` — never a partial
/// `Same`/`Differ` that silently drops the one dependency that couldn't be checked.
///
/// `pub`/`#[doc(hidden)]` for the same reason as its sibling helpers: `tests/it` drives
/// this directly against fixture repos.
#[doc(hidden)]
pub fn path_dependency_comparison(source_dir: &str, build_time: SystemTime) -> PathDepComparison {
    let closure = path_dependency_closure(source_dir);
    if closure.is_empty() {
        return PathDepComparison::NoPathDeps;
    }

    let mut moved = Vec::new();
    for dep_dir in &closure {
        match last_build_input_commit_time(dep_dir) {
            Some(commit_time) if commit_time > build_time => {
                let name = dep_dir
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| dep_dir.display().to_string());
                moved.push(name);
            }
            Some(_) => {}
            None => return PathDepComparison::Unknown,
        }
    }

    if moved.is_empty() {
        PathDepComparison::Same
    } else {
        PathDepComparison::Differ(moved)
    }
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
    let build_inputs = match &live_sha {
        Some(live) if live != stamped_sha && live != "unknown" && stamped_sha != "unknown" => {
            differ_build_inputs(source_dir, stamped_sha, live)
        }
        _ => BuildInputComparison::Unknown,
    };
    // Applies to `self` too, deliberately (see this task's SCOPE NOTE): mev itself
    // path-depends on okf-core, and a per-writer verdict that skipped the closure for
    // `self` would leave the exact measured failure half-fixed.
    let path_deps = if source_dir_exists {
        match writer_build_time(name) {
            Some(build_time) => path_dependency_comparison(source_dir, build_time),
            None => PathDepComparison::Unknown,
        }
    } else {
        // `verdict` returns `NotEvaluable` before ever looking at `path_deps` when the
        // source dir doesn't exist; the value is unused but must still be supplied.
        PathDepComparison::Unknown
    };
    let (status, findings, reason) = verdict(
        stamped_sha,
        live_sha.as_deref(),
        dirty,
        source_dir_exists,
        build_inputs,
        path_deps,
    );
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
        let (status, _findings, reason) = verdict(
            "unknown",
            Some("abc123"),
            "0",
            true,
            BuildInputComparison::Unknown,
            PathDepComparison::NoPathDeps,
        );
        assert_eq!(status, CheckStatus::NotEvaluable);
        assert!(reason.is_some());
    }

    #[test]
    fn not_evaluable_when_dirty_flag_unknown() {
        let (status, _findings, reason) = verdict(
            "abc123",
            Some("abc123"),
            "unknown",
            true,
            BuildInputComparison::Unknown,
            PathDepComparison::NoPathDeps,
        );
        assert_eq!(status, CheckStatus::NotEvaluable);
        assert!(reason.is_some());
    }

    #[test]
    fn not_evaluable_when_source_dir_missing() {
        let (status, _findings, reason) = verdict(
            "abc123",
            Some("abc123"),
            "0",
            false,
            BuildInputComparison::Unknown,
            PathDepComparison::NoPathDeps,
        );
        assert_eq!(status, CheckStatus::NotEvaluable);
        assert!(reason.is_some());
    }

    #[test]
    fn not_evaluable_when_live_sha_unavailable() {
        let (status, _findings, reason) = verdict(
            "abc123",
            None,
            "0",
            true,
            BuildInputComparison::Unknown,
            PathDepComparison::NoPathDeps,
        );
        assert_eq!(status, CheckStatus::NotEvaluable);
        assert!(reason.is_some());
    }

    #[test]
    fn not_evaluable_when_live_sha_literal_unknown() {
        let (status, _findings, reason) = verdict(
            "abc123",
            Some("unknown"),
            "0",
            true,
            BuildInputComparison::Unknown,
            PathDepComparison::NoPathDeps,
        );
        assert_eq!(status, CheckStatus::NotEvaluable);
        assert!(reason.is_some());
    }

    #[test]
    fn drift_when_sha_differs() {
        let (status, findings, reason) = verdict(
            "abc123",
            Some("def456"),
            "0",
            true,
            BuildInputComparison::Differ,
            PathDepComparison::NoPathDeps,
        );
        assert_eq!(status, CheckStatus::Drift);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].contains("abc123"));
        assert!(findings[0].contains("def456"));
        assert!(findings[0].contains("rebuild"));
        assert!(reason.is_none());
    }

    #[test]
    fn drift_when_dirty_even_with_matching_sha() {
        let (status, findings, reason) = verdict(
            "abc123",
            Some("abc123"),
            "1",
            true,
            BuildInputComparison::Unknown,
            PathDepComparison::NoPathDeps,
        );
        assert_eq!(status, CheckStatus::Drift);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].contains("uncommitted"));
        assert!(reason.is_none());
    }

    #[test]
    fn dirty_drift_message_distinct_from_stale_sha_drift_message() {
        let (_status1, stale_findings, _) = verdict(
            "abc123",
            Some("def456"),
            "0",
            true,
            BuildInputComparison::Differ,
            PathDepComparison::NoPathDeps,
        );
        let (_status2, dirty_findings, _) = verdict(
            "abc123",
            Some("abc123"),
            "1",
            true,
            BuildInputComparison::Unknown,
            PathDepComparison::NoPathDeps,
        );
        assert_ne!(stale_findings[0], dirty_findings[0]);
    }

    #[test]
    fn pass_when_sha_matches_and_clean() {
        let (status, findings, reason) = verdict(
            "abc123",
            Some("abc123"),
            "0",
            true,
            BuildInputComparison::Unknown,
            PathDepComparison::NoPathDeps,
        );
        assert_eq!(status, CheckStatus::Pass);
        assert!(findings.is_empty());
        assert!(reason.is_none());
    }

    #[test]
    fn pass_when_sha_differs_but_build_inputs_same() {
        // The core behaviour this ticket adds: SHAs differ, but the build-input
        // comparison says `Same` (e.g. only docs/ changed between the two commits) ->
        // Pass, not Drift.
        let (status, findings, reason) = verdict(
            "abc123",
            Some("def456"),
            "0",
            true,
            BuildInputComparison::Same,
            PathDepComparison::NoPathDeps,
        );
        assert_eq!(status, CheckStatus::Pass);
        // Must be distinguishable from a bare SHA match: assert on the MESSAGE content,
        // not only the status, and name both SHAs plus the non-build explanation.
        assert_eq!(findings.len(), 1);
        assert!(findings[0].contains("abc123"));
        assert!(findings[0].contains("def456"));
        assert!(findings[0].contains("build input") || findings[0].contains("non-build"));
        assert!(reason.is_none());
    }

    #[test]
    fn drift_when_sha_differs_and_build_inputs_unknown() {
        // Absence of a diff answer (unresolvable stamped SHA, git unavailable) must never
        // be read as "no difference" -> still Drift, with a message saying the comparison
        // could not be made.
        let (status, findings, reason) = verdict(
            "abc123",
            Some("def456"),
            "0",
            true,
            BuildInputComparison::Unknown,
            PathDepComparison::NoPathDeps,
        );
        assert_eq!(status, CheckStatus::Drift);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].contains("abc123"));
        assert!(findings[0].contains("def456"));
        assert!(findings[0].contains("could not be made"));
        assert!(reason.is_none());
    }

    #[test]
    fn drift_when_dirty_even_with_sha_differing_and_build_inputs_same() {
        // The other collapse-prone case: dirty==1 still wins even when the SHAs differ
        // AND the build-input comparison independently says `Same` — a careless refactor
        // that checks build_inputs before dirty would wrongly report Pass here.
        let (status, findings, reason) = verdict(
            "abc123",
            Some("def456"),
            "1",
            true,
            BuildInputComparison::Same,
            PathDepComparison::NoPathDeps,
        );
        assert_eq!(status, CheckStatus::Drift);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].contains("uncommitted"));
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
        let config = crate::brain::config::BrainConfig {
            conformance_writers: vec![ConformanceWriter {
                name: "bastion".to_string(),
                repo_path: Some("bastion".to_string()),
            }],
            ..Default::default()
        };
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

    #[test]
    fn writer_outcomes_two_registry_entries_both_named_alongside_self() {
        // SOURCE-level evidence for the un-gateable "config-only edit adds a writer"
        // acceptance criterion (MV.ticket.conformance-writer-registry task 4):
        // `writer_outcomes` takes its writer list purely from its argument, so a
        // second `ConformanceWriter` reaches it exactly the same way as the first —
        // no code path can special-case "the writers the const used to name". This
        // proves the SOURCE behaviour only: that the running-from-this-build binary
        // iterates whatever registry it is handed. It does NOT prove that an already
        // -installed `mev` on PATH would observe a `brain.toml` edit without being
        // rebuilt/reinstalled — that installed-artefact claim is exactly what the
        // block record's acceptance criterion marks `gateable: false` and leaves to
        // manual fixture verification, not an in-repo test.
        let first = fake_writer_script(
            "two_entry_first",
            r#"{"git_sha":"aaaa111","dirty":false,"source_dir":"/tmp/does-not-exist-first"}"#,
            0,
        );
        let second = fake_writer_script(
            "two_entry_second",
            r#"{"git_sha":"bbbb222","dirty":false,"source_dir":"/tmp/does-not-exist-second"}"#,
            0,
        );
        let writers = vec![
            ConformanceWriter {
                name: first.to_str().unwrap().to_string(),
                repo_path: Some("writer-one-repo".to_string()),
            },
            ConformanceWriter {
                name: second.to_str().unwrap().to_string(),
                repo_path: Some("writer-two-repo".to_string()),
            },
        ];

        let (_status, outcomes) = writer_outcomes(&writers);
        let _ = std::fs::remove_file(&first);
        let _ = std::fs::remove_file(&second);

        assert_eq!(outcomes.len(), 3, "self + two registered writers");
        assert_eq!(outcomes[0].name, "self");
        assert_eq!(
            outcomes[1].name,
            first.to_str().unwrap(),
            "writers must appear in registry order"
        );
        assert_eq!(
            outcomes[2].name,
            second.to_str().unwrap(),
            "writers must appear in registry order"
        );
        // Both fake writers report a nonexistent source_dir, so both are named and
        // NotEvaluable — never silently Pass, never dropped from the report.
        assert_eq!(outcomes[1].status, CheckStatus::NotEvaluable);
        assert_eq!(outcomes[2].status, CheckStatus::NotEvaluable);
    }

    // -----------------------------------------------------------------------------------
    // `path_dependency_closure` / `path_dependency_comparison` — the sibling-repo
    // freshness signal this task adds. Fuller fixture coverage (transitive, cycle,
    // positive control against the live SHA-vs-build-time comparison) lives in
    // `tests/it/toolchain_path_dependencies.rs` per this ticket's Task 2; these are the
    // in-file unit tests for the new pure/impure helpers themselves.
    // -----------------------------------------------------------------------------------

    /// Write `Cargo.toml` at `dir` with the given raw `[dependencies]`-table body.
    fn write_manifest(dir: &Path, deps_body: &str) {
        std::fs::write(
            dir.join("Cargo.toml"),
            format!("[package]\nname = \"fixture\"\nversion = \"0.1.0\"\n\n{deps_body}\n"),
        )
        .expect("write fixture Cargo.toml");
    }

    /// `git init` a fixture directory and commit its current contents, returning the
    /// commit's Unix timestamp (`%ct`) — mirroring `differ_build_inputs`'s own git
    /// invocations so a fixture repo test never depends on the ambient `git` config.
    fn init_and_commit(dir: &Path, message: &str) -> u64 {
        let run = |args: &[&str]| {
            let output = crate::shared::git_command()
                .args(args)
                .current_dir(dir)
                .output()
                .expect("spawn git");
            assert!(
                output.status.success(),
                "git {args:?} failed in {}: {}",
                dir.display(),
                String::from_utf8_lossy(&output.stderr)
            );
        };
        if !dir.join(".git").exists() {
            run(&["init", "-q"]);
            run(&["config", "user.email", "test@example.com"]);
            run(&["config", "user.name", "Test"]);
        }
        run(&["add", "."]);
        run(&["commit", "-q", "-m", message]);
        let output = crate::shared::git_command()
            .args(["log", "-1", "--format=%ct"])
            .current_dir(dir)
            .output()
            .expect("spawn git log");
        String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse()
            .expect("commit timestamp")
    }

    #[test]
    fn path_dependency_closure_empty_when_manifest_has_no_path_deps() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_manifest(dir.path(), "[dependencies]\nserde = \"1\"\n");
        let closure = path_dependency_closure(dir.path().to_str().unwrap());
        assert!(closure.is_empty(), "no path deps -> empty closure");
    }

    #[test]
    fn path_dependency_closure_empty_when_manifest_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        // No Cargo.toml at all.
        let closure = path_dependency_closure(dir.path().to_str().unwrap());
        assert!(closure.is_empty());
    }

    #[test]
    fn path_dependency_closure_finds_direct_path_dependency() {
        let dir = tempfile::tempdir().expect("tempdir");
        let writer_dir = dir.path().join("writer");
        let dep_dir = dir.path().join("dep");
        std::fs::create_dir_all(&writer_dir).unwrap();
        std::fs::create_dir_all(&dep_dir).unwrap();
        write_manifest(&writer_dir, "[dependencies]\ndep = { path = \"../dep\" }\n");
        write_manifest(&dep_dir, "[dependencies]\n");

        let closure = path_dependency_closure(writer_dir.to_str().unwrap());
        let expected = dep_dir.canonicalize().unwrap();
        assert_eq!(closure, vec![expected]);
    }

    #[test]
    fn path_dependency_closure_is_transitive() {
        // a -> b -> c: the moved commit lives in c, so the walk must reach it.
        let dir = tempfile::tempdir().expect("tempdir");
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        let c = dir.path().join("c");
        for p in [&a, &b, &c] {
            std::fs::create_dir_all(p).unwrap();
        }
        write_manifest(&a, "[dependencies]\nb = { path = \"../b\" }\n");
        write_manifest(&b, "[dependencies]\nc = { path = \"../c\" }\n");
        write_manifest(&c, "[dependencies]\n");

        let closure = path_dependency_closure(a.to_str().unwrap());
        let names: Vec<String> = closure
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(
            names.contains(&"b".to_string()),
            "must include b: {names:?}"
        );
        assert!(
            names.contains(&"c".to_string()),
            "must reach c transitively: {names:?}"
        );
    }

    #[test]
    fn path_dependency_closure_terminates_on_a_cycle() {
        // a -> b -> a: the visited-set must stop the walk rather than hang.
        let dir = tempfile::tempdir().expect("tempdir");
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        write_manifest(&a, "[dependencies]\nb = { path = \"../b\" }\n");
        write_manifest(&b, "[dependencies]\na = { path = \"../a\" }\n");

        // Bounded by the test harness's own timeout; a regression that removed the
        // visited-set would hang this call rather than return.
        let closure = path_dependency_closure(a.to_str().unwrap());
        let names: std::collections::HashSet<String> = closure
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, std::collections::HashSet::from(["b".to_string()]));
    }

    #[test]
    fn path_dependency_comparison_no_path_deps() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_manifest(dir.path(), "[dependencies]\n");
        let cmp = path_dependency_comparison(dir.path().to_str().unwrap(), SystemTime::now());
        assert_eq!(cmp, PathDepComparison::NoPathDeps);
    }

    #[test]
    fn path_dependency_comparison_same_when_dep_commit_predates_build_time() {
        let dir = tempfile::tempdir().expect("tempdir");
        let writer_dir = dir.path().join("writer");
        let dep_dir = dir.path().join("dep");
        std::fs::create_dir_all(&writer_dir).unwrap();
        std::fs::create_dir_all(&dep_dir).unwrap();
        write_manifest(&writer_dir, "[dependencies]\ndep = { path = \"../dep\" }\n");
        write_manifest(&dep_dir, "[dependencies]\n");
        let commit_secs = init_and_commit(&dep_dir, "initial dep commit");
        let build_time =
            SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(commit_secs + 3600);

        let cmp = path_dependency_comparison(writer_dir.to_str().unwrap(), build_time);
        assert_eq!(
            cmp,
            PathDepComparison::Same,
            "a dependency commit an hour before the writer's build time must not be Differ"
        );
    }

    #[test]
    fn path_dependency_comparison_differ_names_the_moved_dependency() {
        let dir = tempfile::tempdir().expect("tempdir");
        let writer_dir = dir.path().join("writer");
        let dep_dir = dir.path().join("dep");
        std::fs::create_dir_all(&writer_dir).unwrap();
        std::fs::create_dir_all(&dep_dir).unwrap();
        write_manifest(&writer_dir, "[dependencies]\ndep = { path = \"../dep\" }\n");
        write_manifest(&dep_dir, "[dependencies]\n");
        let commit_secs = init_and_commit(&dep_dir, "dep commit after the writer was built");
        // Build time an hour BEFORE the dependency's commit -> Differ.
        let build_time = SystemTime::UNIX_EPOCH
            + std::time::Duration::from_secs(commit_secs.saturating_sub(3600));

        let cmp = path_dependency_comparison(writer_dir.to_str().unwrap(), build_time);
        match cmp {
            PathDepComparison::Differ(names) => {
                assert_eq!(names, vec!["dep".to_string()]);
            }
            other => panic!("expected Differ naming `dep`, got {other:?}"),
        }
    }

    #[test]
    fn path_dep_drift_maps_no_path_deps_and_same_to_no_drift() {
        assert!(path_dep_drift(&PathDepComparison::NoPathDeps).is_none());
        assert!(path_dep_drift(&PathDepComparison::Same).is_none());
    }

    #[test]
    fn path_dep_drift_maps_differ_and_unknown_to_drift_naming_the_dependency() {
        let (status, findings, reason) =
            path_dep_drift(&PathDepComparison::Differ(vec!["okf-core".to_string()]))
                .expect("Differ must drift");
        assert_eq!(status, CheckStatus::Drift);
        assert!(findings[0].contains("okf-core"));
        assert!(reason.is_none());

        let (status, findings, _reason) =
            path_dep_drift(&PathDepComparison::Unknown).expect("Unknown must drift");
        assert_eq!(status, CheckStatus::Drift);
        assert!(!findings.is_empty());
    }

    #[test]
    fn verdict_reports_drift_from_a_moved_path_dependency_even_when_own_sha_matches() {
        // The core behaviour this task adds: the writer's OWN sha/build-input comparison
        // is clean, but a path dependency moved -> still Drift, never a silent Pass.
        let (status, findings, reason) = verdict(
            "abc123",
            Some("abc123"),
            "0",
            true,
            BuildInputComparison::Unknown,
            PathDepComparison::Differ(vec!["okf-core".to_string()]),
        );
        assert_eq!(status, CheckStatus::Drift);
        assert!(findings[0].contains("okf-core"));
        assert!(reason.is_none());
    }

    #[test]
    fn verdict_no_path_deps_or_same_never_turns_a_pass_into_drift() {
        let (status, findings, _) = verdict(
            "abc123",
            Some("abc123"),
            "0",
            true,
            BuildInputComparison::Unknown,
            PathDepComparison::NoPathDeps,
        );
        assert_eq!(status, CheckStatus::Pass);
        assert!(findings.is_empty());

        let (status, findings, _) = verdict(
            "abc123",
            Some("def456"),
            "0",
            true,
            BuildInputComparison::Same,
            PathDepComparison::Same,
        );
        assert_eq!(status, CheckStatus::Pass);
        assert!(!findings.is_empty()); // still carries the own-repo "Same" explanation
    }

    #[test]
    fn verdict_path_dep_drift_never_downgrades_an_existing_drift_to_pass() {
        // An existing Drift verdict (own SHA differs, build inputs Differ) stays Drift
        // regardless of what path_deps says — path deps only ADD Drift, never remove it.
        let (status, findings, _) = verdict(
            "abc123",
            Some("def456"),
            "0",
            true,
            BuildInputComparison::Differ,
            PathDepComparison::NoPathDeps,
        );
        assert_eq!(status, CheckStatus::Drift);
        assert!(findings[0].contains("rebuild"));

        // The dirty branch, too: dirty=1 wins outright.
        let (status, findings, _) = verdict(
            "abc123",
            Some("abc123"),
            "1",
            true,
            BuildInputComparison::Unknown,
            PathDepComparison::Differ(vec!["okf-core".to_string()]),
        );
        assert_eq!(status, CheckStatus::Drift);
        assert!(findings[0].contains("uncommitted"));
    }
}
