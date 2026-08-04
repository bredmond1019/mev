//! Check `backlog-parity` — HQ `planning/backlog.md` `## Active` + `## Promoted` sections
//! vs `planning/state.json` `backlog[]`.
//!
//! `backlog.md` itself documents the rule: those two sections must stay at 1:1 parity
//! with `backlog[]`, while `## Superseded` and `## Shipped` are explicitly record-only
//! and outside the rule. The join key is the exact ticket title (trimmed, internal
//! whitespace collapsed).

use std::path::Path;

use super::{CheckOutcome, CheckStatus, ConformanceCtx, FactSide, compare_sides};

/// Section headings whose `### [YYYY-MM-DD] Title` entries participate in the parity
/// rule. `## Superseded` and `## Shipped` are deliberately excluded.
const PARITY_SECTIONS: &[&str] = &["Active", "Promoted"];

/// Collapse internal whitespace runs to a single space and trim the ends.
fn canonicalize_title(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Parse `backlog.md`'s contents, returning the canonicalized, sorted list of `###
/// [YYYY-MM-DD] Title` headings found under `## Active` and `## Promoted` only.
fn parse_backlog_md(contents: &str) -> Vec<String> {
    let mut titles = Vec::new();
    let mut in_parity_section = false;

    for line in contents.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            // A top-level `## ` heading starts a new section; decide whether its
            // `### [date] Title` children count toward parity.
            let heading = heading.trim();
            in_parity_section = PARITY_SECTIONS.contains(&heading);
            continue;
        }

        if !in_parity_section {
            continue;
        }

        if let Some(rest) = line.strip_prefix("### [") {
            // Expect `YYYY-MM-DD] Title` — split on the first `] ` to isolate the title.
            if let Some(close) = rest.find(']') {
                let after = &rest[close + 1..];
                let title = after.strip_prefix(' ').unwrap_or(after);
                let title = canonicalize_title(title);
                if !title.is_empty() {
                    titles.push(title);
                }
            }
        }
    }

    titles.sort();
    titles
}

/// Find the HQ brain `(StateSource, StateFile)` pair in `ctx.files` — the one whose
/// source is the brain root's own `planning/state.json` and whose `kind == "brain"`.
fn hq_backlog_titles(ctx: &ConformanceCtx) -> Option<Vec<String>> {
    let hq_state_path = ctx.root.join("planning").join("state.json");
    let (_, file) = ctx
        .files
        .iter()
        .find(|(src, file)| src.abs_path == hq_state_path && file.kind == "brain")?;

    let mut titles: Vec<String> = file
        .backlog
        .iter()
        .map(|b| canonicalize_title(&b.title))
        .collect();
    titles.sort();
    Some(titles)
}

/// Run the `backlog-parity` check.
pub fn run(ctx: &ConformanceCtx) -> CheckOutcome {
    let backlog_md_path = ctx.root.join("planning").join("backlog.md");
    if !backlog_md_path.exists() {
        return CheckOutcome {
            status: CheckStatus::NotEvaluable,
            left: None,
            right: None,
            findings: Vec::new(),
            reason: Some(format!(
                "backlog.md not found at {}",
                backlog_md_path.display()
            )),
        };
    }

    let Some(md_titles) = read_backlog_md(&backlog_md_path) else {
        return CheckOutcome {
            status: CheckStatus::NotEvaluable,
            left: None,
            right: None,
            findings: Vec::new(),
            reason: Some(format!(
                "backlog.md could not be read at {}",
                backlog_md_path.display()
            )),
        };
    };

    let Some(json_titles) = hq_backlog_titles(ctx) else {
        return CheckOutcome {
            status: CheckStatus::NotEvaluable,
            left: None,
            right: None,
            findings: Vec::new(),
            reason: Some(
                "no HQ brain (kind=\"brain\") planning/state.json loaded for this root".to_string(),
            ),
        };
    };

    let left = FactSide {
        label: "backlog.md".to_string(),
        source: backlog_md_path.display().to_string(),
        digest: super::digest(&md_titles),
        items: md_titles,
    };
    let right = FactSide {
        label: "state.json backlog[]".to_string(),
        source: ctx
            .root
            .join("planning")
            .join("state.json")
            .display()
            .to_string(),
        digest: super::digest(&json_titles),
        items: json_titles,
    };

    compare_sides(left, right)
}

fn read_backlog_md(path: &Path) -> Option<Vec<String>> {
    let contents = std::fs::read_to_string(path).ok()?;
    Some(parse_backlog_md(&contents))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain::config::BrainConfig;
    use crate::brain::state::{StateFile, StateSource};
    use okf_core::Backlog;

    fn state_source(root: &Path) -> StateSource {
        StateSource {
            repo_slug: "hq".to_string(),
            abs_path: root.join("planning").join("state.json"),
            expected_kind: "brain",
        }
    }

    fn state_file(titles: &[&str]) -> StateFile {
        StateFile {
            repo: "hq".to_string(),
            kind: "brain".to_string(),
            updated: "2026-08-03".to_string(),
            focus: Default::default(),
            tracks: Vec::new(),
            repos: Vec::new(),
            cross_repo: Vec::new(),
            tiers: Vec::new(),
            epics: Vec::new(),
            note: None,
            backlog: titles
                .iter()
                .map(|t| Backlog {
                    slug: t.to_lowercase().replace(' ', "-"),
                    title: t.to_string(),
                    repo: "cross-repo".to_string(),
                    kind: "improvement".to_string(),
                    status: "idea".to_string(),
                    depends_on: Vec::new(),
                    block: None,
                    notes: None,
                    origin: None,
                    created: None,
                    reviewed: None,
                    snoozed_until: None,
                    extra: serde_json::Map::new(),
                })
                .collect(),
            carryover: Vec::new(),
            extra: serde_json::Map::new(),
        }
    }

    fn ctx_with(root: &Path, titles: &[&str]) -> ConformanceCtx {
        ConformanceCtx {
            root: root.to_path_buf(),
            config: BrainConfig::default(),
            files: vec![(state_source(root), state_file(titles))],
        }
    }

    fn write_backlog_md(root: &Path, body: &str) {
        std::fs::create_dir_all(root.join("planning")).unwrap();
        std::fs::write(root.join("planning").join("backlog.md"), body).unwrap();
    }

    #[test]
    fn matching_sides_pass() {
        let root = crate::testsupport::unique_temp_dir("mev-conformance-backlog-match");
        write_backlog_md(
            &root,
            "## Active\n\n### [2026-08-01] Ticket One\nbody\n\n## Promoted\n\n### [2026-08-02] Ticket Two\nbody\n",
        );
        let ctx = ctx_with(&root, &["Ticket One", "Ticket Two"]);

        let outcome = run(&ctx);
        assert_eq!(outcome.status, CheckStatus::Pass);
        assert!(outcome.findings.is_empty());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn title_only_in_markdown_is_drift() {
        let root = crate::testsupport::unique_temp_dir("mev-conformance-backlog-md-only");
        write_backlog_md(
            &root,
            "## Active\n\n### [2026-08-01] Only In Markdown\nbody\n",
        );
        let ctx = ctx_with(&root, &[]);

        let outcome = run(&ctx);
        assert_eq!(outcome.status, CheckStatus::Drift);
        assert!(
            outcome
                .findings
                .iter()
                .any(|f| f.contains("Only In Markdown"))
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn title_only_in_json_is_drift() {
        let root = crate::testsupport::unique_temp_dir("mev-conformance-backlog-json-only");
        write_backlog_md(&root, "## Active\n\n");
        let ctx = ctx_with(&root, &["Only In Json"]);

        let outcome = run(&ctx);
        assert_eq!(outcome.status, CheckStatus::Drift);
        assert!(outcome.findings.iter().any(|f| f.contains("Only In Json")));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn shipped_only_title_does_not_drift() {
        let root = crate::testsupport::unique_temp_dir("mev-conformance-backlog-shipped");
        write_backlog_md(
            &root,
            "## Active\n\n### [2026-08-01] Both Sides\nbody\n\n## Shipped\n\n### [2026-01-01] Shipped Only\nbody\n",
        );
        // "Shipped Only" is intentionally absent from state.json — it must not
        // register as a markdown-only finding because it's under ## Shipped.
        let ctx = ctx_with(&root, &["Both Sides"]);

        let outcome = run(&ctx);
        assert_eq!(outcome.status, CheckStatus::Pass);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_backlog_md_is_not_evaluable() {
        let root = crate::testsupport::unique_temp_dir("mev-conformance-backlog-missing");
        std::fs::create_dir_all(&root).unwrap();
        let ctx = ctx_with(&root, &["Whatever"]);

        let outcome = run(&ctx);
        assert_eq!(outcome.status, CheckStatus::NotEvaluable);
        assert!(outcome.reason.is_some());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn canonicalize_title_collapses_whitespace() {
        assert_eq!(
            canonicalize_title("  a   title   with   spaces  "),
            "a title with spaces"
        );
    }

    #[test]
    fn parse_backlog_md_excludes_superseded_and_shipped() {
        let body = "\
## Active

### [2026-08-01] Active One
body

## Superseded

### [2026-01-01] Superseded One
body

## Shipped

### [2026-01-01] Shipped One
body

## Promoted

### [2026-08-01] Promoted One
body
";
        let titles = parse_backlog_md(body);
        assert_eq!(titles, vec!["Active One", "Promoted One"]);
    }
}
