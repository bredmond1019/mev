//! Check `sibling-rule-coverage` — a rule taught to one function must be taught to its
//! sibling.
//!
//! `derive_brain_focus` and `derive_rollup` both resolve a repo's state file and both
//! must honour the dual-role rule (a registered repo is either a leaf `kind: "project"`
//! or a tier sub-brain root `kind: "brain"`). The first learned the rule; the second kept
//! hard-filtering `kind == "project"` and stayed silently wrong for months — the July
//! backlog ticket describing the symptom was even retired as *shipped* because it was
//! scoped emitter-side only. This check makes the next such drift loud instead of silent.
//!
//! A [`SiblingRule`] declares a set of `members` that must all route through one
//! `shared_helper`, must never re-inline a `forbidden` predicate, and must all be
//! exercised together by one `covering_test`. [`scan_rule`] evaluates a rule against the
//! crate's own source text (read as `Vec<(PathBuf, String)>`) via source-text analysis —
//! no `syn`/AST parsing, deliberately: the defect class is always "one call site was
//! edited and its sibling was not," which is visible directly in the text.
//!
//! [`extract_fn_body`] is the shared primitive: locate `fn <name>` at a word boundary,
//! then return the `{ ... }` block by brace-depth counting that ignores braces inside
//! string literals (honouring `\"` escapes) and `//` line comments. It never guesses — an
//! unfindable or unbalanced function returns `None`, which [`scan_rule`] turns into a
//! `missing-member` finding rather than a silent pass.

use std::path::{Path, PathBuf};

use super::{CheckOutcome, CheckStatus, ConformanceCtx, FactSide};

/// The source directory the running binary was compiled from, stamped by `build.rs` (see
/// `toolchain.rs`). Reused here so `sibling-rule-coverage` locates the same source tree
/// `toolchain-freshness` compares against.
const STAMPED_SOURCE_DIR: &str = env!("MEV_BUILD_SOURCE_DIR");

/// A rule declaring that a set of sibling functions must agree on one shared invariant.
///
/// Registering a new rule is adding one `SiblingRule` literal to [`SIBLING_RULES`] —
/// nothing else changes.
#[derive(Debug, Clone, Copy)]
pub struct SiblingRule {
    /// Stable rule name, e.g. `"dual-role-repo-resolution"`.
    pub name: &'static str,
    /// One sentence describing the invariant, quoted verbatim in every finding.
    pub invariant: &'static str,
    /// The function names that must all agree on the invariant.
    pub members: &'static [&'static str],
    /// The function every member's body must call.
    pub shared_helper: &'static str,
    /// Inline substrings that must not reappear in a member's body (the regression
    /// pattern the shared helper was extracted to eliminate).
    pub forbidden: &'static [&'static str],
    /// The name of a test whose body must mention every member — the "asserted against
    /// BOTH" proof.
    pub covering_test: &'static str,
}

/// The declared table of sibling rules. Populated by later tasks in this ticket.
pub const SIBLING_RULES: &[SiblingRule] = &[SiblingRule {
    name: "dual-role-repo-resolution",
    invariant: "A registered repo resolves to its state file whether that file is \
                kind: \"project\" (leaf) or kind: \"brain\" (tier sub-brain root carrying \
                its own authored tracks[]).",
    members: &["derive_rollup", "derive_brain_focus"],
    shared_helper: "resolve_repo_state_file",
    forbidden: &["f.kind == \"project\""],
    covering_test: "dual_role_rule_holds_for_both_resolvers",
}];

/// The four failure modes `scan_rule` can report for a rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindingKind {
    /// A declared member function no longer exists in the source.
    MissingMember,
    /// A member's body does not mention the rule's `shared_helper`.
    HelperNotCalled,
    /// A member's body contains one of the rule's `forbidden` substrings.
    ForbiddenInlined,
    /// The `covering_test` is absent, or present but does not mention every member.
    TestNotCovering,
}

impl FindingKind {
    /// The stable, kebab-case name used in reports and tests.
    pub fn as_str(self) -> &'static str {
        match self {
            FindingKind::MissingMember => "missing-member",
            FindingKind::HelperNotCalled => "helper-not-called",
            FindingKind::ForbiddenInlined => "forbidden-inlined",
            FindingKind::TestNotCovering => "test-not-covering",
        }
    }
}

/// One divergence surfaced by [`scan_rule`]: names the rule, the member (or the covering
/// test, for `TestNotCovering`), and quotes the rule's invariant verbatim in `message`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub kind: FindingKind,
    pub rule: &'static str,
    pub member: &'static str,
    pub message: String,
}

impl Finding {
    fn new(kind: FindingKind, rule: &SiblingRule, member: &'static str, detail: &str) -> Self {
        Finding {
            kind,
            rule: rule.name,
            member,
            message: format!(
                "[{}] sibling rule '{}' ({}): {}",
                kind.as_str(),
                rule.name,
                rule.invariant,
                detail
            ),
        }
    }
}

/// Locate `fn <fn_name>` in `source` at a word boundary (so `fn foo` does not match
/// `fn foobar`, nor a `foo` embedded in a longer identifier before `fn`), then return the
/// `{ ... }` body by brace-depth counting from the first `{` after the match.
///
/// Brace counting ignores braces inside double-quoted string literals (honouring `\"`
/// escapes) and inside `//` line comments. Returns `None` — never a wrong slice — when the
/// function is not found, no opening brace follows it, or the braces never balance.
pub fn extract_fn_body<'a>(source: &'a str, fn_name: &str) -> Option<&'a str> {
    let pattern = format!("fn {fn_name}");
    let bytes = source.as_bytes();
    let mut search_start = 0usize;

    loop {
        if search_start >= source.len() {
            return None;
        }
        let found_rel = source[search_start..].find(pattern.as_str())?;
        let match_start = search_start + found_rel;
        let match_end = match_start + pattern.len();

        let before_ok = match match_start.checked_sub(1) {
            Some(idx) => {
                let c = bytes[idx];
                !(c.is_ascii_alphanumeric() || c == b'_')
            }
            None => true,
        };
        let after_ok = match bytes.get(match_end) {
            Some(&c) => !(c.is_ascii_alphanumeric() || c == b'_'),
            None => true,
        };

        if before_ok && after_ok {
            let open_idx = match_end + source[match_end..].find('{')?;
            return extract_balanced(source, open_idx);
        }

        search_start = match_start + 1;
    }
}

/// Given `source` and the byte index of an opening `{`, return the slice from that brace
/// to its matching close (inclusive), skipping braces inside string literals and line
/// comments. Returns `None` if the braces never balance.
fn extract_balanced(source: &str, open_idx: usize) -> Option<&str> {
    let bytes = source.as_bytes();
    if bytes.get(open_idx) != Some(&b'{') {
        return None;
    }

    let mut depth: i32 = 0;
    let mut i = open_idx;
    let mut in_string = false;
    let mut in_line_comment = false;

    while i < bytes.len() {
        let c = bytes[i];

        if in_line_comment {
            if c == b'\n' {
                in_line_comment = false;
            }
            i += 1;
            continue;
        }

        if in_string {
            if c == b'\\' {
                // Skip the escaped character too (handles `\"` correctly).
                i += if i + 1 < bytes.len() { 2 } else { 1 };
                continue;
            }
            if c == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }

        if c == b'"' {
            in_string = true;
            i += 1;
            continue;
        }
        if c == b'/' && bytes.get(i + 1) == Some(&b'/') {
            in_line_comment = true;
            i += 2;
            continue;
        }

        if c == b'{' {
            depth += 1;
        } else if c == b'}' {
            depth -= 1;
            if depth == 0 {
                return Some(&source[open_idx..=i]);
            }
        }
        i += 1;
    }

    None
}

/// Evaluate one [`SiblingRule`] against every source file, returning every finding
/// (empty when the rule fully holds). `sources` is every `.rs` file's `(path, contents)`.
pub fn scan_rule(rule: &SiblingRule, sources: &[(PathBuf, String)]) -> Vec<Finding> {
    let mut findings = Vec::new();

    for &member in rule.members {
        let body = sources
            .iter()
            .find_map(|(_, src)| extract_fn_body(src, member));

        let Some(body) = body else {
            findings.push(Finding::new(
                FindingKind::MissingMember,
                rule,
                member,
                &format!("member function `{member}` not found in source"),
            ));
            continue;
        };

        if !body.contains(rule.shared_helper) {
            findings.push(Finding::new(
                FindingKind::HelperNotCalled,
                rule,
                member,
                &format!(
                    "member `{member}` does not call shared helper `{}`",
                    rule.shared_helper
                ),
            ));
        }

        for &forbidden in rule.forbidden {
            if body.contains(forbidden) {
                findings.push(Finding::new(
                    FindingKind::ForbiddenInlined,
                    rule,
                    member,
                    &format!("member `{member}` re-inlines forbidden pattern `{forbidden}`"),
                ));
            }
        }
    }

    let test_body = sources
        .iter()
        .find_map(|(_, src)| extract_fn_body(src, rule.covering_test));

    match test_body {
        None => {
            findings.push(Finding::new(
                FindingKind::TestNotCovering,
                rule,
                rule.covering_test,
                &format!("covering test `{}` not found", rule.covering_test),
            ));
        }
        Some(body) => {
            let missing: Vec<&str> = rule
                .members
                .iter()
                .filter(|m| !body.contains(**m))
                .copied()
                .collect();
            if !missing.is_empty() {
                findings.push(Finding::new(
                    FindingKind::TestNotCovering,
                    rule,
                    rule.covering_test,
                    &format!(
                        "covering test `{}` does not mention member(s): {}",
                        rule.covering_test,
                        missing.join(", ")
                    ),
                ));
            }
        }
    }

    findings
}

/// Recursively collect every `.rs` file under `dir` into `out` as `(path, contents)`.
/// Unreadable individual files are skipped (not fatal — a genuinely missing source
/// directory is handled by the caller before this is ever invoked).
fn collect_rs_files(dir: &Path, out: &mut Vec<(PathBuf, String)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs")
            && let Ok(contents) = std::fs::read_to_string(&path)
        {
            out.push((path, contents));
        }
    }
}

/// Locate the source tree via `source_dir` (the `MEV_BUILD_SOURCE_DIR` build stamp),
/// reading every `.rs` file under its `src/` and `tests/` subdirectories.
///
/// Returns `None` — never a partial/guessed reading — when `source_dir` is `"unknown"`
/// or does not exist as a directory. This is the sole gate that turns an unreachable
/// source tree into `NotEvaluable` rather than a (possibly empty, falsely-passing)
/// source list.
fn discover_sources(source_dir: &str) -> Option<Vec<(PathBuf, String)>> {
    if source_dir == "unknown" {
        return None;
    }
    let root = Path::new(source_dir);
    if !root.is_dir() {
        return None;
    }

    let mut files = Vec::new();
    for sub in ["src", "tests"] {
        let sub_dir = root.join(sub);
        if sub_dir.is_dir() {
            collect_rs_files(&sub_dir, &mut files);
        }
    }
    Some(files)
}

/// Evaluate [`SIBLING_RULES`] against the source tree rooted at `source_dir`. Pulled out
/// from [`run`] as a pure function of `source_dir` so it is directly unit-testable
/// (including against a nonexistent directory) without needing to fake a
/// `ConformanceCtx`.
fn evaluate(source_dir: &str) -> CheckOutcome {
    let Some(sources) = discover_sources(source_dir) else {
        return CheckOutcome {
            status: CheckStatus::NotEvaluable,
            left: None,
            right: None,
            findings: Vec::new(),
            reason: Some(format!(
                "sibling-rule-coverage: source tree unreachable at MEV_BUILD_SOURCE_DIR \
                 ({source_dir})"
            )),
        };
    };

    let mut all_findings: Vec<Finding> = Vec::new();
    for rule in SIBLING_RULES {
        all_findings.extend(scan_rule(rule, &sources));
    }

    let status = if all_findings.is_empty() {
        CheckStatus::Pass
    } else {
        CheckStatus::Drift
    };

    let left = FactSide {
        label: "declared sibling rules".to_string(),
        source: "SIBLING_RULES".to_string(),
        digest: super::digest(
            &SIBLING_RULES
                .iter()
                .map(|r| format!("{}={}", r.name, r.members.join(",")))
                .collect::<Vec<_>>(),
        ),
        items: SIBLING_RULES
            .iter()
            .map(|r| format!("{}={}", r.name, r.members.join(",")))
            .collect(),
    };

    let right_items: Vec<String> = SIBLING_RULES
        .iter()
        .map(|rule| {
            let n = all_findings.iter().filter(|f| f.rule == rule.name).count();
            if n == 0 {
                format!("{}=ok", rule.name)
            } else {
                format!("{}={n} findings", rule.name)
            }
        })
        .collect();
    let right = FactSide {
        label: "observed source state".to_string(),
        source: format!("{source_dir}/src, {source_dir}/tests"),
        digest: super::digest(&right_items),
        items: right_items,
    };

    let findings: Vec<String> = all_findings.into_iter().map(|f| f.message).collect();

    CheckOutcome {
        status,
        left: Some(left),
        right: Some(right),
        findings,
        reason: None,
    }
}

/// Run the `sibling-rule-coverage` check against the source tree stamped at build time
/// via `MEV_BUILD_SOURCE_DIR`.
pub fn run(_ctx: &ConformanceCtx) -> CheckOutcome {
    evaluate(STAMPED_SOURCE_DIR)
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- extract_fn_body ---------------------------------------------------------

    #[test]
    fn extract_fn_body_plain_function() {
        let src = "fn foo(x: i32) -> i32 {\n    x + 1\n}\n";
        let body = extract_fn_body(src, "foo").expect("body found");
        assert_eq!(body, "{\n    x + 1\n}");
    }

    #[test]
    fn extract_fn_body_ignores_brace_inside_string_literal() {
        let src = r#"fn foo() {
    let s = "a { b";
    let t = 1;
}
"#;
        let body = extract_fn_body(src, "foo").expect("body found");
        assert!(body.contains("a { b"));
        assert!(body.trim_end().ends_with('}'));
        // Exactly one real closing brace terminates the body — the string's `{` did not
        // throw off the depth count.
        assert_eq!(body.matches("let t = 1;").count(), 1);
    }

    #[test]
    fn extract_fn_body_ignores_brace_inside_line_comment() {
        let src = "fn foo() {\n    // a stray } here\n    let y = 2;\n}\n";
        let body = extract_fn_body(src, "foo").expect("body found");
        assert!(body.contains("stray } here"));
        assert!(body.contains("let y = 2;"));
        assert!(body.trim_end().ends_with('}'));
    }

    #[test]
    fn extract_fn_body_respects_escaped_quotes_in_strings() {
        let src = r#"fn foo() {
    let s = "she said \"hi { there\"";
    let z = 3;
}
"#;
        let body = extract_fn_body(src, "foo").expect("body found");
        assert!(body.contains("let z = 3;"));
    }

    #[test]
    fn extract_fn_body_does_not_match_longer_identifier() {
        let src = "fn foobar() {\n    1\n}\n";
        assert!(extract_fn_body(src, "foo").is_none());
    }

    #[test]
    fn extract_fn_body_finds_exact_name_after_longer_identifier() {
        let src = "fn foobar() {\n    1\n}\nfn foo() {\n    2\n}\n";
        let body = extract_fn_body(src, "foo").expect("body found");
        assert!(body.contains('2'));
        assert!(!body.contains('1'));
    }

    #[test]
    fn extract_fn_body_none_when_not_found() {
        let src = "fn bar() {\n    1\n}\n";
        assert!(extract_fn_body(src, "foo").is_none());
    }

    #[test]
    fn extract_fn_body_none_when_unbalanced() {
        let src = "fn foo() {\n    let x = 1;\n";
        assert!(extract_fn_body(src, "foo").is_none());
    }

    #[test]
    fn extract_fn_body_none_when_no_opening_brace() {
        let src = "fn foo();\n";
        assert!(extract_fn_body(src, "foo").is_none());
    }

    // --- scan_rule -----------------------------------------------------------------

    fn test_rule() -> SiblingRule {
        SiblingRule {
            name: "test-rule",
            invariant: "members must call helper and never re-inline forbidden",
            members: &["alpha", "beta"],
            shared_helper: "shared_helper",
            forbidden: &["banned_pattern"],
            covering_test: "covers_both",
        }
    }

    fn src(path: &str, contents: &str) -> (PathBuf, String) {
        (PathBuf::from(path), contents.to_string())
    }

    #[test]
    fn scan_rule_clean_case_has_no_findings() {
        let sources = vec![src(
            "lib.rs",
            "fn alpha() {\n    shared_helper();\n}\nfn beta() {\n    shared_helper();\n}\n\
             fn covers_both() {\n    alpha();\n    beta();\n}\n",
        )];
        let findings = scan_rule(&test_rule(), &sources);
        assert!(findings.is_empty(), "expected no findings: {findings:?}");
    }

    #[test]
    fn scan_rule_reports_missing_member() {
        let sources = vec![src(
            "lib.rs",
            "fn alpha() {\n    shared_helper();\n}\n\
             fn covers_both() {\n    alpha();\n    beta();\n}\n",
        )];
        let findings = scan_rule(&test_rule(), &sources);
        assert!(
            findings
                .iter()
                .any(|f| f.kind == FindingKind::MissingMember && f.member == "beta")
        );
    }

    #[test]
    fn scan_rule_reports_helper_not_called() {
        let sources = vec![src(
            "lib.rs",
            "fn alpha() {\n    shared_helper();\n}\n\
             fn beta() {\n    do_something_else();\n}\n\
             fn covers_both() {\n    alpha();\n    beta();\n}\n",
        )];
        let findings = scan_rule(&test_rule(), &sources);
        assert!(
            findings
                .iter()
                .any(|f| f.kind == FindingKind::HelperNotCalled && f.member == "beta")
        );
    }

    #[test]
    fn scan_rule_reports_forbidden_inlined() {
        let sources = vec![src(
            "lib.rs",
            "fn alpha() {\n    shared_helper();\n}\n\
             fn beta() {\n    shared_helper();\n    banned_pattern();\n}\n\
             fn covers_both() {\n    alpha();\n    beta();\n}\n",
        )];
        let findings = scan_rule(&test_rule(), &sources);
        assert!(
            findings
                .iter()
                .any(|f| f.kind == FindingKind::ForbiddenInlined && f.member == "beta")
        );
    }

    #[test]
    fn scan_rule_reports_test_not_covering_when_test_missing() {
        let sources = vec![src(
            "lib.rs",
            "fn alpha() {\n    shared_helper();\n}\nfn beta() {\n    shared_helper();\n}\n",
        )];
        let findings = scan_rule(&test_rule(), &sources);
        assert!(
            findings
                .iter()
                .any(|f| f.kind == FindingKind::TestNotCovering)
        );
    }

    #[test]
    fn scan_rule_reports_test_not_covering_when_test_misses_a_member() {
        let sources = vec![src(
            "lib.rs",
            "fn alpha() {\n    shared_helper();\n}\nfn beta() {\n    shared_helper();\n}\n\
             fn covers_both() {\n    alpha();\n}\n",
        )];
        let findings = scan_rule(&test_rule(), &sources);
        let f = findings
            .iter()
            .find(|f| f.kind == FindingKind::TestNotCovering)
            .expect("expected a test-not-covering finding");
        assert!(f.message.contains("beta"));
    }

    #[test]
    fn scan_rule_findings_quote_invariant_verbatim() {
        let sources = vec![src("lib.rs", "fn alpha() {}\n")];
        let findings = scan_rule(&test_rule(), &sources);
        assert!(
            findings
                .iter()
                .all(|f| f.message.contains(test_rule().invariant))
        );
    }

    #[test]
    fn finding_kind_as_str_matches_spec_names() {
        assert_eq!(FindingKind::MissingMember.as_str(), "missing-member");
        assert_eq!(FindingKind::HelperNotCalled.as_str(), "helper-not-called");
        assert_eq!(FindingKind::ForbiddenInlined.as_str(), "forbidden-inlined");
        assert_eq!(FindingKind::TestNotCovering.as_str(), "test-not-covering");
    }

    #[test]
    fn sibling_rules_registry_has_dual_role_rule_registered() {
        // Task 2 registers the dual-role-repo-resolution rule; task 4 adds the second.
        assert!(
            SIBLING_RULES
                .iter()
                .any(|r| r.name == "dual-role-repo-resolution")
        );
    }

    #[test]
    fn dual_role_rule_has_all_six_fields_populated() {
        let rule = SIBLING_RULES
            .iter()
            .find(|r| r.name == "dual-role-repo-resolution")
            .expect("dual-role-repo-resolution registered");
        assert!(!rule.invariant.is_empty());
        assert_eq!(rule.members, &["derive_rollup", "derive_brain_focus"]);
        assert_eq!(rule.shared_helper, "resolve_repo_state_file");
        assert_eq!(rule.forbidden, &["f.kind == \"project\""]);
        assert_eq!(
            rule.covering_test,
            "dual_role_rule_holds_for_both_resolvers"
        );
    }

    // --- discover_sources / evaluate ------------------------------------------------

    #[test]
    fn discover_sources_none_for_unknown_source_dir() {
        assert!(discover_sources("unknown").is_none());
    }

    #[test]
    fn discover_sources_none_for_missing_directory() {
        assert!(discover_sources("/this/path/definitely/does/not/exist/mev-test").is_none());
    }

    #[test]
    fn evaluate_reports_not_evaluable_for_nonexistent_source_dir() {
        let outcome = evaluate("/this/path/definitely/does/not/exist/mev-test");
        assert_eq!(outcome.status, CheckStatus::NotEvaluable);
        assert!(outcome.left.is_none());
        assert!(outcome.right.is_none());
        assert!(outcome.reason.is_some());
    }

    #[test]
    fn evaluate_reports_not_evaluable_for_unknown_stamp() {
        let outcome = evaluate("unknown");
        assert_eq!(outcome.status, CheckStatus::NotEvaluable);
    }

    #[test]
    fn evaluate_never_reports_pass_when_source_unreachable() {
        let outcome = evaluate("/another/nonexistent/mev-test-path");
        assert_ne!(outcome.status, CheckStatus::Pass);
    }

    #[test]
    fn run_uses_real_stamped_source_dir_without_panicking() {
        let ctx = ConformanceCtx {
            root: std::path::PathBuf::from("."),
            config: crate::brain::config::BrainConfig::default(),
            files: Vec::new(),
        };
        let outcome = run(&ctx);
        // Whatever the verdict, it must never silently claim Pass on an empty/failed
        // read — either Pass with populated sides, Drift with findings, or NotEvaluable.
        match outcome.status {
            CheckStatus::Pass => {
                assert!(outcome.left.is_some());
                assert!(outcome.right.is_some());
            }
            CheckStatus::Drift => assert!(!outcome.findings.is_empty()),
            CheckStatus::NotEvaluable => assert!(outcome.reason.is_some()),
        }
    }
}
