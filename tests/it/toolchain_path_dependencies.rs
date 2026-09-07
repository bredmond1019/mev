//! Integration coverage for `toolchain-freshness`'s path-dependency closure (Task 2 of
//! `MV.ticket.toolchain-freshness-must-follow-path-dependencies`).
//!
//! Task 1 added `path_dependency_closure` / `path_dependency_comparison` /
//! `PathDepComparison` to `src/brain/conformance/toolchain.rs`, `pub`/`#[doc(hidden)]`
//! precisely so this crate can drive them directly against throwaway fixture repos —
//! matching `differ_build_inputs`'s existing precedent. Every fixture here is a real
//! `git init` repo built in a tempdir; nothing asserts against the live fleet, whose
//! SHAs move under the test.
//!
//! Six fixtures, matching the task's own enumeration:
//!   1. direct dependency with a build-input commit newer than the writer's build time -> Drift, naming the dep
//!   2. same shape, but the dep's newer commit touches only a non-build-input path -> NOT Drift
//!   3. no path dependencies at all -> identical (NoPathDeps) verdict
//!   4. transitive closure a -> b -> c, moved commit in c -> Differ names c
//!   5. a dependency cycle a -> b -> a -> terminates instead of hanging
//!   6. positive control: path dependency present but unmoved since build time -> Same

use mev::brain::conformance::toolchain::{
    PathDepComparison, path_dependency_closure, path_dependency_comparison,
    resolve_build_input_paths,
};
use std::path::Path;
use std::time::{Duration, SystemTime};

/// Run `git` with `args` inside `dir`, panicking with stderr on failure. Goes through
/// `mev::testsupport::git_command()` — a plain `Command::new("git")` inherits `GIT_DIR`
/// and friends from the parent process, which override `-C`/`current_dir` and make the
/// fixture operate on the WRONG repository (see that helper's own doc comment; this bit
/// mev's own suite when run from inside `hooks/pre-push`).
fn git(dir: &Path, args: &[&str]) {
    let output = mev::testsupport::git_command()
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
}

/// `git init` a fixture directory (idempotent) and commit its current working-tree
/// contents, returning the new commit's Unix timestamp (`%ct`).
fn init_and_commit(dir: &Path, message: &str) -> u64 {
    if !dir.join(".git").exists() {
        git(dir, &["init", "-q"]);
        git(dir, &["config", "user.email", "test@example.com"]);
        git(dir, &["config", "user.name", "Test"]);
    }
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", message]);
    let output = mev::testsupport::git_command()
        .args(["log", "-1", "--format=%ct"])
        .current_dir(dir)
        .output()
        .expect("spawn git log");
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .expect("commit timestamp")
}

fn write_manifest(dir: &Path, deps_body: &str) {
    std::fs::create_dir_all(dir).expect("create fixture dir");
    std::fs::write(
        dir.join("Cargo.toml"),
        format!("[package]\nname = \"fixture\"\nversion = \"0.1.0\"\n\n{deps_body}\n"),
    )
    .expect("write fixture Cargo.toml");
}

fn build_time_secs(secs: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
}

/// Fixture 1: a direct path dependency with a build-input commit AFTER the writer's
/// build time must be Drift, and the finding must name the dependency (`dep`).
#[test]
fn direct_dependency_with_newer_build_input_commit_is_drift_naming_the_dep() {
    let root = mev::testsupport::unique_temp_dir("mev-toolchain-path-dep-direct-drift");
    let writer_dir = root.join("writer");
    let dep_dir = root.join("dep");
    write_manifest(&writer_dir, "[dependencies]\ndep = { path = \"../dep\" }\n");
    write_manifest(&dep_dir, "[dependencies]\n");

    // Writer is considered "built" before the dependency's src/ change lands.
    let build_time = build_time_secs(1_000_000);
    std::fs::write(dep_dir.join("src.rs"), "// placeholder build input\n").ok();
    std::fs::create_dir_all(dep_dir.join("src")).unwrap();
    std::fs::write(dep_dir.join("src").join("lib.rs"), "// v2\n").unwrap();
    let commit_secs = init_and_commit(&dep_dir, "dep src change after writer was built");
    assert!(
        commit_secs > 1_000_000,
        "fixture commit clock must land after the writer's fixed build time"
    );

    let default_paths = resolve_build_input_paths(&[]);
    let cmp = path_dependency_comparison(writer_dir.to_str().unwrap(), build_time, &default_paths);
    match cmp {
        PathDepComparison::Differ(names) => {
            assert_eq!(
                names,
                vec!["dep".to_string()],
                "finding must name the dep repo"
            );
        }
        other => panic!("expected Differ naming `dep`, got {other:?}"),
    }
}

/// Fixture 2: the dependency's newer commit touches only a non-build-input path (a docs
/// file, not `src/`) — this must NOT read as Drift, mirroring `BuildInputComparison`'s
/// existing repo-local behaviour where a non-build-input diff reports `Same`.
#[test]
fn dependency_commit_touching_only_docs_is_not_drift() {
    let root = mev::testsupport::unique_temp_dir("mev-toolchain-path-dep-docs-only");
    let writer_dir = root.join("writer");
    let dep_dir = root.join("dep");
    write_manifest(&writer_dir, "[dependencies]\ndep = { path = \"../dep\" }\n");
    write_manifest(&dep_dir, "[dependencies]\n");
    let build_secs = init_and_commit(
        &dep_dir,
        "initial dep commit (this IS a build input: Cargo.toml)",
    );
    let build_time = build_time_secs(build_secs + 3600);

    // A commit strictly after the writer's build time, but touching only a docs file —
    // not one of BUILD_INPUT_PATHS (`src/`, `crates/`, `tests/`, `build.rs`,
    // `Cargo.toml`, `Cargo.lock`, `.cargo/`).
    std::fs::write(dep_dir.join("README.md"), "docs only change\n").unwrap();
    init_and_commit(&dep_dir, "docs-only change, not a build input");

    let default_paths = resolve_build_input_paths(&[]);
    let cmp = path_dependency_comparison(writer_dir.to_str().unwrap(), build_time, &default_paths);
    assert_eq!(
        cmp,
        PathDepComparison::Same,
        "a non-build-input change in the dependency must not read as Differ"
    );
}

/// Fixture 3: a writer with no path dependencies at all behaves exactly as before —
/// the regression guard for every writer not part of a path-dep graph.
#[test]
fn writer_with_no_path_dependencies_is_unaffected() {
    let root = mev::testsupport::unique_temp_dir("mev-toolchain-path-dep-none");
    write_manifest(&root, "[dependencies]\nserde = \"1\"\n");

    let closure = path_dependency_closure(root.to_str().unwrap());
    assert!(closure.is_empty(), "no path deps -> empty closure");

    let default_paths = resolve_build_input_paths(&[]);
    let cmp = path_dependency_comparison(root.to_str().unwrap(), SystemTime::now(), &default_paths);
    assert_eq!(cmp, PathDepComparison::NoPathDeps);
}

/// Fixture 4: a transitive closure `a -> b -> c` where the moved build-input commit
/// lives only in `c`. Proves the walk reaches beyond one level.
#[test]
fn transitive_closure_reaches_the_leaf_and_reports_its_drift() {
    let root = mev::testsupport::unique_temp_dir("mev-toolchain-path-dep-transitive");
    let a = root.join("a");
    let b = root.join("b");
    let c = root.join("c");
    write_manifest(&a, "[dependencies]\nb = { path = \"../b\" }\n");
    write_manifest(&b, "[dependencies]\nc = { path = \"../c\" }\n");
    write_manifest(&c, "[dependencies]\n");

    let build_time = build_time_secs(1_000_000);
    // `b` needs its own commit so `last_build_input_commit_time` can answer for it too —
    // an uncommitted dependency dir reads as `Unknown`, not `Same`, per that helper's
    // documented doctrine, and `Unknown` would swallow the real `c` signal below.
    init_and_commit(&b, "b initial commit, well before the writer was built");
    let commit_secs = init_and_commit(&c, "c src change after writer was built");
    assert!(commit_secs > 1_000_000);

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

    let default_paths = resolve_build_input_paths(&[]);
    let cmp = path_dependency_comparison(a.to_str().unwrap(), build_time, &default_paths);
    match cmp {
        PathDepComparison::Differ(names) => {
            assert!(
                names.contains(&"c".to_string()),
                "must name c, got {names:?}"
            );
        }
        other => panic!("expected Differ naming `c`, got {other:?}"),
    }
}

/// Fixture 5: a dependency cycle `a -> b -> a` must terminate rather than hang. Bounded
/// via a `Command`-level timeout is unnecessary here since the call is synchronous and
/// in-process — a regression that removed the visited-set would hang the WHOLE test
/// process, which is exactly what the assertion below is designed to make loud and
/// immediate rather than silent.
#[test]
fn dependency_cycle_terminates_instead_of_hanging() {
    let root = mev::testsupport::unique_temp_dir("mev-toolchain-path-dep-cycle");
    let a = root.join("a");
    let b = root.join("b");
    write_manifest(&a, "[dependencies]\nb = { path = \"../b\" }\n");
    write_manifest(&b, "[dependencies]\na = { path = \"../a\" }\n");

    let closure = path_dependency_closure(a.to_str().unwrap());
    let names: std::collections::HashSet<String> = closure
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        names,
        std::collections::HashSet::from(["b".to_string()]),
        "cycle must terminate and report exactly the one reachable sibling"
    );

    // Comparison must also terminate and produce a real answer, not hang.
    let default_paths = resolve_build_input_paths(&[]);
    let cmp = path_dependency_comparison(a.to_str().unwrap(), SystemTime::now(), &default_paths);
    assert!(
        matches!(
            cmp,
            PathDepComparison::Same | PathDepComparison::Differ(_) | PathDepComparison::Unknown
        ),
        "comparison over a cyclic closure must resolve to a real variant, got {cmp:?}"
    );
}

/// Fixture 6 (POSITIVE CONTROL): a path dependency IS present but has NOT moved since
/// the writer's build time -> Same. Without this, a change that reported Drift
/// unconditionally would still satisfy every other assertion in this file.
#[test]
fn unmoved_path_dependency_is_pass_not_drift() {
    let root = mev::testsupport::unique_temp_dir("mev-toolchain-path-dep-positive-control");
    let writer_dir = root.join("writer");
    let dep_dir = root.join("dep");
    write_manifest(&writer_dir, "[dependencies]\ndep = { path = \"../dep\" }\n");
    write_manifest(&dep_dir, "[dependencies]\n");
    let commit_secs = init_and_commit(&dep_dir, "dep commit, well before the writer was built");
    let build_time = build_time_secs(commit_secs + 3600);

    let default_paths = resolve_build_input_paths(&[]);
    let cmp = path_dependency_comparison(writer_dir.to_str().unwrap(), build_time, &default_paths);
    assert_eq!(
        cmp,
        PathDepComparison::Same,
        "an unmoved path dependency must report Same, not Differ"
    );
}
