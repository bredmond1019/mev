//! Shared helper functions used by multiple validators.
//!
//! These are string-manipulation and format-checking utilities that do not belong to any single
//! consumer (learn-ai, OKF, …) and carry no domain types. Each helper is `pub(crate)` so all
//! internal modules can import it without exposing it as part of the crate's public API.

/// Extract the raw YAML text from a leading `---\n…\n---` frontmatter block.
///
/// Returns `None` when the file does not start with `---` (no frontmatter), or when the
/// closing `---` fence is never found (unterminated block).
pub(crate) fn extract_frontmatter(contents: &str) -> Option<&str> {
    let rest = contents.strip_prefix("---")?;
    // Accept `---\n` or `---\r\n` as the opening fence line.
    let rest = rest
        .strip_prefix('\n')
        .or_else(|| rest.strip_prefix("\r\n"))?;

    // Special case: closing fence immediately follows opening fence (empty frontmatter).
    for empty_pat in ["---\n", "---\r\n", "---"] {
        if rest.starts_with(empty_pat) {
            return Some("");
        }
    }

    // General case: find the closing `---` on its own line (preceded by a newline).
    for end_pat in ["\n---\n", "\n---\r\n", "\n---"] {
        if let Some(yaml_end) = rest.find(end_pat) {
            return Some(&rest[..yaml_end]);
        }
    }

    None
}

/// Return the original string borrow if it is non-blank, or `None` if absent/whitespace-only.
pub(crate) fn non_empty(value: &Option<String>) -> Option<&str> {
    match value {
        Some(s) if !s.trim().is_empty() => Some(s),
        _ => None,
    }
}

/// `true` if `s` matches `^[a-z0-9]+(-[a-z0-9]+)*$` (kebab-case), without the `regex` crate.
pub(crate) fn is_kebab_case(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    // Each hyphen-delimited segment must be non-empty and all `[a-z0-9]` — this rejects
    // leading/trailing hyphens and `--` runs (which produce an empty segment when split).
    s.split('-')
        .all(|seg| !seg.is_empty() && seg.chars().all(|c| matches!(c, 'a'..='z' | '0'..='9')))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_frontmatter_helper() {
        // Normal case.
        let body = "---\ntitle: T\n---\nbody";
        assert_eq!(extract_frontmatter(body), Some("title: T"));

        // Unterminated: returns None.
        let unterminated = "---\ntitle: T\n";
        assert_eq!(extract_frontmatter(unterminated), None);

        // No leading fence: returns None.
        let no_fence = "title: T\n---\n";
        assert_eq!(extract_frontmatter(no_fence), None);

        // Empty frontmatter block.
        let empty_block = "---\n---\nbody";
        assert_eq!(extract_frontmatter(empty_block), Some(""));
    }

    #[test]
    fn kebab_case_helper() {
        assert!(is_kebab_case("intro-to-mcp"));
        assert!(is_kebab_case("abc123"));
        assert!(!is_kebab_case("Intro"));
        assert!(!is_kebab_case("-leading"));
        assert!(!is_kebab_case("trailing-"));
        assert!(!is_kebab_case("double--hyphen"));
        assert!(!is_kebab_case("under_score"));
        assert!(!is_kebab_case(""));
    }

    #[test]
    fn non_empty_helper() {
        assert_eq!(non_empty(&Some("hello".to_string())), Some("hello"));
        assert_eq!(non_empty(&Some("  ".to_string())), None);
        assert_eq!(non_empty(&None), None);
        assert_eq!(non_empty(&Some("".to_string())), None);
        // Trimmed value is returned as-is (not trimmed), but check it's truthy.
        assert_eq!(non_empty(&Some("  x  ".to_string())), Some("  x  "));
    }
}

/// The repository-scoping environment variables git exports to the hooks it runs. Any child
/// `git` inherits them, and they **override `-C`** — so a call meant for one path silently
/// operates on the hook's repository instead.
pub(crate) const GIT_REPO_ENV_VARS: [&str; 9] = [
    "GIT_DIR",
    "GIT_COMMON_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_NAMESPACE",
    "GIT_PREFIX",
    "GIT_CEILING_DIRECTORIES",
];

/// Remove every variable in [`GIT_REPO_ENV_VARS`] from `cmd`'s environment plan.
///
/// Order matters: this must be applied **after** anything that sets those variables on `cmd`,
/// because `Command::env` and `Command::env_remove` are recorded in call order.
pub(crate) fn strip_git_repo_env(cmd: &mut std::process::Command) {
    for var in GIT_REPO_ENV_VARS {
        cmd.env_remove(var);
    }
}

/// Build a `git` [`Command`] with every inherited `GIT_*` repository variable removed.
///
/// **Always use this instead of `Command::new("git")`.** Git exports `GIT_DIR` (and, depending
/// on the hook, `GIT_INDEX_FILE`, `GIT_WORK_TREE`, `GIT_OBJECT_DIRECTORY`, …) to every hook it
/// runs. A child `git` inherits them and they **override both `-C` and `current_dir`**, so the
/// call silently operates on the *hook's* repository rather than the path asked about.
///
/// Measured 2026-08-21: mev's own `cargo test` passes 26/26 from a shell and fails 8 of the
/// same 26 when `GIT_DIR` is set — `git ["init"]` failing inside a fresh `tempdir`, and
/// `is_linked_worktree` answering about the wrong tree. Because `hooks/pre-push` stage 2 runs
/// `cargo test`, mev's test suite could never pass from inside a push, which blocked
/// `MV.17.A`'s PR with eight failures unrelated to the branch. Stage 3's consumer gate hit the
/// production half of the same bug, reporting bastion and engine-rs as NOT-EVALUABLE with
/// `git status` exiting 128.
pub(crate) fn git_command() -> std::process::Command {
    let mut cmd = std::process::Command::new("git");
    strip_git_repo_env(&mut cmd);
    cmd
}

#[cfg(test)]
mod git_command_tests {
    use std::process::Command;

    const DECOY: &str = "/nonexistent/decoy.git";

    /// Simulate what a git hook hands a child process — `GIT_DIR` naming another repository —
    /// and assert that stripping it lets `git -C <fresh tempdir> init` succeed.
    ///
    /// The `.env(...)` call comes FIRST and the strip second, which is the real ordering: the
    /// variable arrives from the environment and the strip removes it. Reversing the two makes
    /// this test pass while proving nothing, because `env` after `env_remove` re-adds it.
    #[test]
    fn stripping_git_repo_env_survives_an_inherited_git_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut cmd = Command::new("git");
        cmd.env("GIT_DIR", DECOY);
        super::strip_git_repo_env(&mut cmd);
        let status = cmd
            .arg("-C")
            .arg(dir.path())
            .arg("init")
            .status()
            .expect("git must be on PATH");
        assert!(
            status.success(),
            "git -C <tempdir> init must succeed once GIT_DIR is stripped"
        );
        assert!(
            dir.path().join(".git").exists(),
            "init must have created .git in the -C path, not at GIT_DIR"
        );
    }

    /// Positive control for the test above. Without it, a green result there could mean the
    /// environment never mattered rather than that the strip works — which is exactly the
    /// mistake that produced this bug's first, wrong fix.
    #[test]
    fn plain_command_is_broken_by_an_inherited_git_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let status = Command::new("git")
            .env("GIT_DIR", DECOY)
            .arg("-C")
            .arg(dir.path())
            .arg("init")
            .status()
            .expect("git must be on PATH");
        assert!(
            !status.success(),
            "control failed: a plain git Command no longer breaks under an inherited GIT_DIR, \
             so stripping_git_repo_env_survives_an_inherited_git_dir proves nothing"
        );
    }

    /// `git_command()` must apply the strip itself, so a caller gets the protection without
    /// having to remember the second call.
    #[test]
    fn git_command_plans_a_removal_for_every_repo_var() {
        let cmd = super::git_command();
        let removed: Vec<String> = cmd
            .get_envs()
            .filter(|(_, v)| v.is_none())
            .map(|(k, _)| k.to_string_lossy().to_string())
            .collect();
        for var in super::GIT_REPO_ENV_VARS {
            assert!(
                removed.iter().any(|r| r == var),
                "git_command() must plan a removal for {var}; planned: {removed:?}"
            );
        }
    }
}
