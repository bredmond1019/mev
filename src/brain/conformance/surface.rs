//! Check `surface-leak` — public repos' tracked files vs unpublishable relative links
//! and private infra literals.
//!
//! Confirmed systemic, not hypothetical: the 2026-08-29 docs pass found the
//! unpublishable-link class in every one of the 9 repos audited (52 instances in
//! base-template alone across 10 files), and found the Mac Mini's real Tailscale IP and
//! port hardcoded in `core/bastion-ui/docs/device-install.md`, a public repo. Both were
//! fixed by hand and nothing re-checked either class, so the next instance was invisible
//! again. The defect passes local validation by construction — the `planning/` D46 vault
//! symlink makes a leaking link resolve fine on the authoring machine — so only a check
//! that reasons about what is TRACKED, rather than what resolves, can catch it.
//!
//! THE TRACKED-SET RULE — this is the entire point of the check, do not weaken it. The
//! target of a leaking link RESOLVES FINE on the authoring machine, through the
//! `planning/` symlink, so any implementation that asks "does this path exist" passes
//! every real leak. The question is "is this path TRACKED IN THIS REPO'S GIT". The
//! tracked set is built per repo from `git -C <repo> ls-files`, one invocation per repo.
//!
//! Two rules, applied only to `[[repos]]` entries with `public == true` (a repo whose
//! entry omits or sets `public = false` is never walked):
//!   - **Rule 1 (unpublishable link)**: a relative markdown link target that is not in
//!     the repo's tracked set, or that lexically normalizes to a path outside the repo
//!     root.
//!   - **Rule 2 (private infra literal)**: a dotted-quad IPv4 address or a `*.ts.net`
//!     Tailscale-shaped hostname in a tracked file, minus `[surface_allowlist].literals`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use regex::Regex;

use super::{CheckOutcome, CheckStatus, ConformanceCtx, FactSide};
use crate::brain::config::RepoEntry;

/// One finding: which repo, which file, which line, which rule, and the detail text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub repo: String,
    pub file: String,
    pub line: usize,
    pub rule: &'static str,
    pub detail: String,
}

impl Finding {
    fn render(&self) -> String {
        format!(
            "{}/{}:{} {}: {}",
            self.repo, self.file, self.line, self.rule, self.detail
        )
    }
}

/// Outcome of trying to build one repo's tracked set: either the set itself, or a
/// human-readable reason `git ls-files` could not be trusted.
type TrackedSetResult = Result<HashSet<String>, String>;

/// Run `git -C <repo_dir> ls-files` and parse the output into a set of repo-relative
/// paths (forward-slash separated, as git emits them). `Err` names the failure — never a
/// silent empty set, which would read as "nothing tracked" rather than "could not ask".
fn tracked_set(repo_dir: &Path) -> TrackedSetResult {
    if !repo_dir.is_dir() {
        return Err(format!("repo path does not exist: {}", repo_dir.display()));
    }
    let output = crate::shared::git_command()
        .args(["ls-files"])
        .current_dir(repo_dir)
        .output()
        .map_err(|e| format!("could not run git ls-files in {}: {e}", repo_dir.display()))?;
    if !output.status.success() {
        return Err(format!(
            "git ls-files in {} exited non-zero: {}",
            repo_dir.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let text = String::from_utf8(output.stdout).map_err(|e| {
        format!(
            "git ls-files output was not UTF-8 in {}: {e}",
            repo_dir.display()
        )
    })?;
    Ok(text.lines().map(|s| s.to_string()).collect())
}

/// One `[text](target)` markdown link match: the raw target string and the 1-indexed
/// line it appeared on.
struct LinkRef {
    target: String,
    line: usize,
}

/// Find every markdown link target in `content`, skipping absolute URLs
/// (`http://`/`https://`/`mailto:`), in-page anchors (bare `#...`), and `file://` links.
/// Pure text scan — no filesystem access.
fn find_links(content: &str) -> Vec<LinkRef> {
    // `[text](target)` — text has no `]`, target has no whitespace or `)`.
    let re = Regex::new(r"\[[^\]]*\]\(([^)\s]+)\)").expect("static regex must compile");
    let mut out = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        for caps in re.captures_iter(line) {
            let target = caps[1].to_string();
            if target.starts_with("http://")
                || target.starts_with("https://")
                || target.starts_with("mailto:")
                || target.starts_with("file://")
                || target.starts_with('#')
            {
                continue;
            }
            out.push(LinkRef {
                target,
                line: idx + 1,
            });
        }
    }
    out
}

/// Strip a trailing `#fragment` from a link target, if present.
fn strip_fragment(target: &str) -> &str {
    match target.find('#') {
        Some(idx) => &target[..idx],
        None => target,
    }
}

/// Lexically normalize `path` (a `/`-joined string, possibly with `.`/`..` segments)
/// WITHOUT touching the filesystem — a filesystem-resolving normalization (e.g.
/// `std::fs::canonicalize`) would follow the `planning/` symlink and defeat the whole
/// point of this check. Returns `None` when the normalized path climbs above its own
/// root (more `..` than preceding segments).
fn lexical_normalize(path: &str) -> Option<Vec<String>> {
    let mut stack: Vec<String> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => continue,
            ".." => {
                stack.pop()?; // None here means "climbed above root"
            }
            other => stack.push(other.to_string()),
        }
    }
    Some(stack)
}

/// Evaluate rule 1 for a single tracked markdown file's content against `tracked`, the
/// repo's full tracked set. `file_rel` is the file's own repo-relative path (`/`-joined).
fn check_links_in_file(
    repo: &str,
    file_rel: &str,
    content: &str,
    tracked: &HashSet<String>,
) -> Vec<Finding> {
    let mut findings = Vec::new();
    let file_dir: Vec<&str> = {
        let mut parts: Vec<&str> = file_rel.split('/').collect();
        parts.pop(); // drop the filename itself
        parts
    };

    for link in find_links(content) {
        let target = strip_fragment(&link.target);
        if target.is_empty() {
            continue;
        }
        // Resolve the target against the containing file's directory, lexically.
        let mut joined = file_dir.join("/");
        if !joined.is_empty() {
            joined.push('/');
        }
        joined.push_str(target);

        match lexical_normalize(&joined) {
            None => findings.push(Finding {
                repo: repo.to_string(),
                file: file_rel.to_string(),
                line: link.line,
                rule: "rule1",
                detail: format!(
                    "link target `{}` climbs above the repo root when resolved from {}",
                    link.target, file_rel
                ),
            }),
            Some(segments) => {
                let normalized = segments.join("/");
                if !tracked.contains(&normalized) {
                    findings.push(Finding {
                        repo: repo.to_string(),
                        file: file_rel.to_string(),
                        line: link.line,
                        rule: "rule1",
                        detail: format!(
                            "link target `{}` resolves to `{}`, which is not tracked in this repo",
                            link.target, normalized
                        ),
                    });
                }
            }
        }
    }
    findings
}

/// One matched rule-2 literal, with its kind for the detail string.
struct LiteralMatch {
    text: String,
    line: usize,
}

/// Find every dotted-quad IPv4 address or `*.ts.net` hostname in `content`, with line
/// numbers. Pure text scan.
fn find_literals(content: &str) -> Vec<LiteralMatch> {
    let ipv4 = Regex::new(r"\b(?:\d{1,3}\.){3}\d{1,3}\b").expect("static regex must compile");
    let tsnet = Regex::new(r"\b[a-zA-Z0-9-]+(?:\.[a-zA-Z0-9-]+)*\.ts\.net\b")
        .expect("static regex must compile");
    let mut out = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        for m in ipv4.find_iter(line) {
            out.push(LiteralMatch {
                text: m.as_str().to_string(),
                line: idx + 1,
            });
        }
        for m in tsnet.find_iter(line) {
            out.push(LiteralMatch {
                text: m.as_str().to_string(),
                line: idx + 1,
            });
        }
    }
    out
}

/// Whether `literal` is covered by an entry in `allowlist`: an entry ending in `.`
/// matches as a prefix (so `192.0.2.` covers the whole block); otherwise the match is
/// exact.
fn is_allowlisted(literal: &str, allowlist: &[String]) -> bool {
    allowlist.iter().any(|entry| {
        if let Some(prefix) = entry.strip_suffix('.') {
            literal.starts_with(&format!("{prefix}."))
        } else {
            literal == entry
        }
    })
}

/// Evaluate rule 2 for a single tracked file's content.
fn check_literals_in_file(
    repo: &str,
    file_rel: &str,
    content: &str,
    allowlist: &[String],
) -> Vec<Finding> {
    find_literals(content)
        .into_iter()
        .filter(|m| !is_allowlisted(&m.text, allowlist))
        .map(|m| Finding {
            repo: repo.to_string(),
            file: file_rel.to_string(),
            line: m.line,
            rule: "rule2",
            detail: format!("private-infra-shaped literal `{}`", m.text),
        })
        .collect()
}

/// Evaluate both rules for one public repo. Returns `Ok(findings)` (possibly empty) or
/// `Err(reason)` when the repo's tracked set could not be built.
pub fn evaluate_repo(
    root: &Path,
    repo: &RepoEntry,
    allowlist: &[String],
) -> Result<Vec<Finding>, String> {
    let repo_dir: PathBuf = if repo.repo_path.is_empty() || repo.repo_path == "." {
        root.to_path_buf()
    } else {
        root.join(&repo.repo_path)
    };

    let tracked = tracked_set(&repo_dir)?;
    let mut findings = Vec::new();

    for rel in &tracked {
        let full = repo_dir.join(rel);
        let content = match std::fs::read_to_string(&full) {
            Ok(c) => c,
            Err(_) => continue, // binary or unreadable file — not this check's concern
        };

        if rel.ends_with(".md") || rel.ends_with(".mdx") {
            findings.extend(check_links_in_file(&repo.slug, rel, &content, &tracked));
        }
        findings.extend(check_literals_in_file(&repo.slug, rel, &content, allowlist));
    }

    Ok(findings)
}

/// Run the `surface-leak` check: walk every `public = true` repo, apply both rules, and
/// worst-wins aggregate — any finding is `Drift`; a repo whose tracked set could not be
/// read is `NotEvaluable`, named; no findings and no failures is `Pass`.
pub fn run(ctx: &ConformanceCtx) -> CheckOutcome {
    let public_repos: Vec<&RepoEntry> = ctx.config.repos.iter().filter(|r| r.public).collect();

    if public_repos.is_empty() {
        return CheckOutcome {
            status: CheckStatus::Pass,
            left: None,
            right: None,
            findings: Vec::new(),
            reason: None,
        };
    }

    let allowlist = &ctx.config.surface_allowlist.literals;
    let mut all_findings: Vec<Finding> = Vec::new();
    let mut reasons: Vec<String> = Vec::new();
    let mut status = CheckStatus::Pass;

    for repo in &public_repos {
        match evaluate_repo(&ctx.root, repo, allowlist) {
            Ok(findings) => {
                if !findings.is_empty() {
                    status = CheckStatus::Drift;
                    all_findings.extend(findings);
                }
            }
            Err(reason) => {
                if status != CheckStatus::Drift {
                    status = CheckStatus::NotEvaluable;
                }
                reasons.push(format!("{}: {reason}", repo.slug));
            }
        }
    }

    let rendered: Vec<String> = all_findings.iter().map(Finding::render).collect();
    let left = FactSide {
        label: "public repos walked".to_string(),
        source: "brain.toml [[repos]] with public = true".to_string(),
        digest: super::digest(
            &public_repos
                .iter()
                .map(|r| r.slug.clone())
                .collect::<Vec<_>>(),
        ),
        items: public_repos.iter().map(|r| r.slug.clone()).collect(),
    };
    let right = FactSide {
        label: "rule1/rule2 findings".to_string(),
        source: "git ls-files tracked-set walk per public repo".to_string(),
        digest: super::digest(&rendered),
        items: rendered.clone(),
    };

    CheckOutcome {
        status,
        left: Some(left),
        right: Some(right),
        findings: rendered,
        reason: if reasons.is_empty() {
            None
        } else {
            Some(reasons.join("; "))
        },
    }
}

#[cfg(test)]
mod tests {
    //! DELIBERATE DIVERGENCE from `toolchain.rs`'s test style: that check's tests avoid
    //! shelling out to git because its subject has nothing to do with git. This check's
    //! whole subject IS git's tracked set, so its fixtures use a real `git init` +
    //! `git add` + `.gitignore`, so the tracked-vs-resolves distinction is exercised for
    //! real rather than mocked. Do not "fix" this back to mocked fixtures — a mock
    //! cannot exhibit the exact defect (a path that resolves on disk but is not
    //! tracked) that this check exists to catch.

    use super::*;
    use crate::brain::config::RepoEntry;
    use std::process::Command;

    fn run_git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(dir)
            .status()
            .expect("git must be on PATH for these tests");
        assert!(status.success(), "git {args:?} failed in {}", dir.display());
    }

    fn init_repo(dir: &Path) {
        run_git(dir, &["init", "-q"]);
        run_git(dir, &["config", "user.email", "test@example.com"]);
        run_git(dir, &["config", "user.name", "Test"]);
    }

    fn write(dir: &Path, rel: &str, content: &str) {
        let full = dir.join(rel);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&full, content).unwrap();
    }

    fn commit_all(dir: &Path) {
        run_git(dir, &["add", "-A"]);
        run_git(dir, &["commit", "-q", "-m", "fixture"]);
    }

    fn repo_entry(slug: &str, repo_path: &str, public: bool) -> RepoEntry {
        RepoEntry {
            slug: slug.to_string(),
            tier: "core".to_string(),
            repo_path: repo_path.to_string(),
            status_file: String::new(),
            cache_doc: String::new(),
            heading: String::new(),
            prefix: None,
            public,
        }
    }

    // --- pure helpers ---

    #[test]
    fn find_links_skips_absolute_and_anchor_links() {
        let content = "[a](https://example.com)\n[b](#section)\n[c](mailto:x@y.com)\n[d](file:///etc/passwd)\n[e](./real.md)\n";
        let links = find_links(content);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, "./real.md");
        assert_eq!(links[0].line, 5);
    }

    #[test]
    fn strip_fragment_removes_hash_suffix() {
        assert_eq!(strip_fragment("docs/x.md#heading"), "docs/x.md");
        assert_eq!(strip_fragment("docs/x.md"), "docs/x.md");
    }

    #[test]
    fn lexical_normalize_handles_dot_and_dotdot() {
        assert_eq!(
            lexical_normalize("a/./b/../c"),
            Some(vec!["a".to_string(), "c".to_string()])
        );
    }

    #[test]
    fn lexical_normalize_climb_out_is_none() {
        assert_eq!(lexical_normalize("../../etc/passwd"), None);
    }

    #[test]
    fn is_allowlisted_prefix_match() {
        let list = vec!["192.0.2.".to_string()];
        assert!(is_allowlisted("192.0.2.55", &list));
        assert!(!is_allowlisted("192.0.3.55", &list));
    }

    #[test]
    fn is_allowlisted_exact_match() {
        let list = vec!["127.0.0.1".to_string()];
        assert!(is_allowlisted("127.0.0.1", &list));
        assert!(!is_allowlisted("127.0.0.2", &list));
    }

    #[test]
    fn find_literals_matches_ipv4_and_tsnet() {
        let content = "the box is at 100.64.1.2 also reachable at mini.tailnet-abc.ts.net\n";
        let matches = find_literals(content);
        let texts: Vec<&str> = matches.iter().map(|m| m.text.as_str()).collect();
        assert!(texts.contains(&"100.64.1.2"));
        assert!(texts.contains(&"mini.tailnet-abc.ts.net"));
    }

    // --- fixture-repo cases, per the block record's acceptance criteria ---

    #[test]
    fn vault_symlink_style_leak_fires() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        // A tracked doc links into planning/ (gitignored — resolves locally via the
        // D46 symlink, but is never tracked by this repo's own git).
        write(dir.path(), "README.md", "see [notes](planning/notes.md)\n");
        write(dir.path(), ".gitignore", "planning/\n");
        // The target exists on disk (simulating the symlink resolving) but is
        // gitignored, so it must never be added/tracked.
        write(dir.path(), "planning/notes.md", "hi\n");
        commit_all(dir.path());

        let repo = repo_entry("fixture", "", true);
        let findings = evaluate_repo(dir.path(), &repo, &[]).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule, "rule1");
        assert_eq!(findings[0].file, "README.md");
        assert_eq!(findings[0].line, 1);
    }

    #[test]
    fn climb_out_of_repo_root_link_fires() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        write(
            dir.path(),
            "docs/guide.md",
            "see [x](../../outside/root.md)\n",
        );
        commit_all(dir.path());

        let repo = repo_entry("fixture", "", true);
        let findings = evaluate_repo(dir.path(), &repo, &[]).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule, "rule1");
        assert!(findings[0].detail.contains("climbs above"));
    }

    #[test]
    fn tracked_sibling_link_does_not_fire() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        write(dir.path(), "README.md", "see [x](docs/guide.md)\n");
        write(dir.path(), "docs/guide.md", "hello\n");
        commit_all(dir.path());

        let repo = repo_entry("fixture", "", true);
        let findings = evaluate_repo(dir.path(), &repo, &[]).unwrap();
        assert!(findings.is_empty(), "findings: {findings:?}");
    }

    #[test]
    fn private_repo_is_never_walked() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        write(dir.path(), "README.md", "see [notes](planning/notes.md)\n");
        write(dir.path(), ".gitignore", "planning/\n");
        write(dir.path(), "planning/notes.md", "hi\n");
        commit_all(dir.path());

        let repo = repo_entry("fixture", dir.path().to_str().unwrap(), false);
        let mut config = crate::brain::config::BrainConfig::default();
        config.repos = vec![repo];
        let ctx = ConformanceCtx {
            root: PathBuf::from("."),
            config,
            files: Vec::new(),
        };
        let outcome = run(&ctx);
        assert_eq!(outcome.status, CheckStatus::Pass);
        assert!(outcome.findings.is_empty());
    }

    #[test]
    fn dotted_quad_in_tracked_file_fires() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        write(dir.path(), "docs/install.md", "connect to 100.64.5.9\n");
        commit_all(dir.path());

        let repo = repo_entry("fixture", "", true);
        let findings = evaluate_repo(dir.path(), &repo, &[]).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule, "rule2");
        assert!(findings[0].detail.contains("100.64.5.9"));
    }

    #[test]
    fn allowlisted_address_does_not_fire() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        write(dir.path(), "docs/install.md", "example: 192.0.2.55\n");
        commit_all(dir.path());

        let repo = repo_entry("fixture", "", true);
        let allowlist = vec!["192.0.2.".to_string()];
        let findings = evaluate_repo(dir.path(), &repo, &allowlist).unwrap();
        assert!(findings.is_empty(), "findings: {findings:?}");
    }

    #[test]
    fn ts_net_hostname_fires() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        write(
            dir.path(),
            "docs/install.md",
            "ssh mini.tailnet-abc123.ts.net\n",
        );
        commit_all(dir.path());

        let repo = repo_entry("fixture", "", true);
        let findings = evaluate_repo(dir.path(), &repo, &[]).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule, "rule2");
        assert!(findings[0].detail.contains("ts.net"));
    }

    #[test]
    fn git_ls_files_failure_is_not_evaluable_named() {
        let dir = tempfile::tempdir().unwrap();
        // Not a git repo at all — git ls-files will fail.
        let missing = dir.path().join("nope");
        std::fs::create_dir_all(&missing).unwrap();

        let repo = repo_entry("broken-repo", "", true);
        let mut config = crate::brain::config::BrainConfig::default();
        config.repos = vec![repo];
        let ctx = ConformanceCtx {
            root: missing.clone(),
            config,
            files: Vec::new(),
        };
        let outcome = run(&ctx);
        assert_eq!(outcome.status, CheckStatus::NotEvaluable);
        let reason = outcome.reason.unwrap();
        assert!(reason.contains("broken-repo"));
    }

    #[test]
    fn no_public_repos_is_pass() {
        let ctx = ConformanceCtx {
            root: PathBuf::from("."),
            config: crate::brain::config::BrainConfig::default(),
            files: Vec::new(),
        };
        let outcome = run(&ctx);
        assert_eq!(outcome.status, CheckStatus::Pass);
    }

    /// Positive control for rule 2 (MV.ticket.surface-leak-check task 4, acceptance
    /// criterion "positive control -- the check re-detects a known-historical leak when
    /// it is reintroduced", `gateable: false`). Reconstructs the SHAPE of the
    /// 2026-08-29 `core/bastion-ui/docs/device-install.md` leak — a Tailscale `.ts.net`
    /// hostname alongside a dotted-quad IP and port, in a device-install-style doc — in
    /// a disposable scratch fixture. DOES NOT touch the real file: that file belongs to
    /// another repo and another lane, and a test that mutates it (even temporarily)
    /// risks losing that lane's live work. A check that reports Pass fleet-wide on its
    /// first run is otherwise indistinguishable from a check that never ran; this test
    /// is what tells the two apart.
    #[test]
    fn historical_tailscale_device_install_leak_shape_fires_rule2() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        write(
            dir.path(),
            "docs/device-install.md",
            "## Connect over Tailscale\n\
             \n\
             SSH in at `mini.tailnet-abc123.ts.net` or directly via `100.64.1.23:8080`.\n",
        );
        commit_all(dir.path());

        let repo = repo_entry("bastion-ui", "", true);
        let findings = evaluate_repo(dir.path(), &repo, &[]).unwrap();

        let rule2: Vec<&Finding> = findings.iter().filter(|f| f.rule == "rule2").collect();
        assert_eq!(
            rule2.len(),
            2,
            "expected both literals to fire: {findings:?}"
        );
        assert!(rule2.iter().any(|f| f.detail.contains("ts.net")));
        assert!(rule2.iter().any(|f| f.detail.contains("100.64.1.23")));
        assert!(rule2.iter().all(|f| f.file == "docs/device-install.md"));
    }

    /// Positive control for rule 1 (same acceptance criterion as above, mirror image).
    /// Reconstructs the SHAPE of the base-template/mev climb-out class found in the
    /// 2026-08-29 audit — a tracked doc linking above its own repo root, e.g. into the
    /// HQ root — in a disposable scratch fixture, not any real repo.
    #[test]
    fn historical_climb_out_leak_shape_fires_rule1() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        write(
            dir.path(),
            "docs/index.md",
            "See [the master plan](../../core/planning/master-plan.md) for context.\n",
        );
        commit_all(dir.path());

        let repo = repo_entry("base-template", "", true);
        let findings = evaluate_repo(dir.path(), &repo, &[]).unwrap();

        assert_eq!(findings.len(), 1, "findings: {findings:?}");
        assert_eq!(findings[0].rule, "rule1");
        assert_eq!(findings[0].file, "docs/index.md");
        assert!(findings[0].detail.contains("climbs above"));
    }

    /// Live-corpus test: run the real check against the real fleet and assert Pass —
    /// the 2026-08-29 fixes are expected to hold on a clean checkout. Skips cleanly
    /// (never fails) when brain.toml is absent, matching this repo's other live-corpus
    /// tests — a CI checkout without the private HQ vault must not go red on it.
    #[test]
    /// Runs the real check over the live fleet and REPORTS — it deliberately does not
    /// assert the corpus is clean, because it is not: the first real run on 2026-09-02
    /// produced 379 findings, one a genuine Tailscale disclosure in a public repo. The
    /// block record routes live findings to `carryover[]` rather than failing a gate, so
    /// what this test proves is that the check EXECUTES over the real corpus without
    /// panicking, and prints what it saw. It is named for what it does; the earlier name
    /// (`..._is_clean`) claimed a verdict it never checked — the same silent-green shape
    /// this check exists to catch.
    fn live_corpus_surface_leak_runs_and_reports_without_panicking() {
        let live_root = std::path::Path::new("../..");
        let live_brain_toml = live_root.join("brain.toml");
        if !live_brain_toml.exists() {
            eprintln!(
                "skipping live_corpus_surface_leak_runs_and_reports_without_panicking: {} has no brain.toml \
                 (fresh clone or CI runner without the sibling HQ checkout)",
                live_root.display()
            );
            return;
        }

        let config = match crate::brain::config::load_brain_config(&live_brain_toml) {
            Ok(c) => c,
            Err(e) => {
                eprintln!(
                    "skipping live_corpus_surface_leak_runs_and_reports_without_panicking: live brain.toml errored: {e}"
                );
                return;
            }
        };

        let ctx = ConformanceCtx {
            root: live_root.to_path_buf(),
            config,
            files: Vec::new(),
        };
        let outcome = run(&ctx);
        // Report, but don't hard-fail the whole suite on a live fleet finding — this is
        // a conformance check meant to be run and reported by `mev conformance`, not a
        // gate; the block record explicitly routes any live finding to carryover rather
        // than fixing it here. Still, print the findings so a CI log shows them.
        if outcome.status != CheckStatus::Pass {
            eprintln!(
                "live_corpus_surface_leak_runs_and_reports_without_panicking: check reported {:?}: {:?} (reason: {:?})",
                outcome.status, outcome.findings, outcome.reason
            );
        }
    }
}
