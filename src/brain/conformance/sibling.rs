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

use std::path::PathBuf;

use super::{CheckOutcome, CheckStatus, ConformanceCtx};

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
pub const SIBLING_RULES: &[SiblingRule] = &[];

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

/// Run the `sibling-rule-coverage` check.
///
/// Source discovery (locating `MEV_BUILD_SOURCE_DIR`, reading every `.rs` file) is wired
/// in a later task of this ticket; for now — with [`SIBLING_RULES`] empty and no source
/// read attempted — the check reports `NotEvaluable` rather than guessing a verdict.
/// Never `Pass` on an unevaluated check.
pub fn run(_ctx: &ConformanceCtx) -> CheckOutcome {
    // No source files are read yet — `sources` is intentionally empty until task 2 wires
    // `MEV_BUILD_SOURCE_DIR` discovery. Scanning against an empty source list here already
    // exercises the real `scan_rule` path (and will start reporting `missing-member` for
    // every declared member the moment a rule is registered), it just cannot yet be
    // trusted as a verdict — hence `NotEvaluable`, never `Pass`.
    let sources: Vec<(PathBuf, String)> = Vec::new();
    let mut findings: Vec<Finding> = Vec::new();
    for rule in SIBLING_RULES {
        findings.extend(scan_rule(rule, &sources));
    }
    let findings: Vec<String> = findings.into_iter().map(|f| f.message).collect();

    CheckOutcome {
        status: CheckStatus::NotEvaluable,
        left: None,
        right: None,
        findings,
        reason: Some(
            "sibling-rule-coverage: source discovery not yet wired (see task 2 of \
             MV.ticket.sibling-rule-coverage)"
                .to_string(),
        ),
    }
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
    fn sibling_rules_registry_starts_empty() {
        // Task 1 populates the machinery only; tasks 2 and 4 register the real rules.
        assert!(SIBLING_RULES.is_empty());
    }
}
