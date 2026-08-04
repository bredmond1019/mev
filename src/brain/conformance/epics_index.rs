//! Check `epics-index-parity` — the HQ `epics[]` registry vs
//! `core/planning/epics/index.md`.
//!
//! `index.md`'s own prose admits the coupling: *"This column is hand-maintained — when
//! you change a status in `planning/state.json`, change it here too."* A hand-synced
//! surface by construction.
//!
//! The join key is each index row's markdown link TARGET, resolved relative to
//! `core/planning/epics/` into a brain-root-relative path, then matched against the
//! registry's `epics[].plan` values — **not** a hardcoded `core/planning/epics/<slug>.md`
//! assumption. 16 of 17 live epics point inside that directory, but
//! `bullet-proof-software` points at `planning/bullet-proof-software/roadmap.md`; a
//! hardcoded join would emit a false missing-doc finding for it. The link stem is used as
//! the slug fallback only when a row's resolved target matches no `plan` value.

use std::path::Path;

use super::{CheckOutcome, CheckStatus, ConformanceCtx, FactSide, compare_sides};
use crate::brain::state::Epic;

/// Repo-relative directory every index-row link target is resolved against.
const EPICS_DIR: &[&str] = &["core", "planning", "epics"];

/// Canonicalize an epic's status for the comparison — absent becomes the empty string so
/// it still participates in the item (and would visibly mismatch an authored status).
fn canonical_status(status: &Option<String>) -> String {
    status.clone().unwrap_or_default()
}

/// Resolve a markdown link target against [`EPICS_DIR`], honoring `..` segments, and
/// return the resulting brain-root-relative path as a `/`-joined string (independent of
/// the host OS's path separator, since it is compared against JSON string values).
fn resolve_link_target(target: &str) -> String {
    let mut components: Vec<&str> = EPICS_DIR.to_vec();
    for part in target.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                components.pop();
            }
            other => components.push(other),
        }
    }
    components.join("/")
}

/// Extract the `(text, target)` pair out of a markdown link cell, e.g.
/// `[bastion-tui.md](bastion-tui.md)` -> `Some("bastion-tui.md")` for the target.
fn parse_link_target(cell: &str) -> Option<&str> {
    let open = cell.find("](")? + 2;
    let close = cell[open..].find(')')? + open;
    Some(&cell[open..close])
}

/// The link target's file stem (no extension), used as the slug fallback when no
/// registry `plan` matches the resolved target.
fn link_stem(target: &str) -> String {
    Path::new(target)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| target.to_string())
}

/// Split a markdown table row into trimmed cells (the leading/trailing `|` are stripped
/// first, so `cols[0]` is the first real column).
fn table_cells(line: &str) -> Vec<&str> {
    line.trim()
        .trim_start_matches('|')
        .trim_end_matches('|')
        .split('|')
        .map(str::trim)
        .collect()
}

/// Parse `index.md`'s table rows into canonical `"<slug>=<status>"` items, sorted.
///
/// Only rows whose first column is a markdown link (`| [...` after trimming) are table
/// data rows — the header row and the `|---|---|` separator are skipped naturally.
fn parse_index_md(contents: &str, epics: &[Epic]) -> Vec<String> {
    let mut items = Vec::new();

    for line in contents.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("| [") {
            continue;
        }

        let cols = table_cells(trimmed);
        if cols.len() < 3 {
            continue;
        }

        let Some(target) = parse_link_target(cols[0]) else {
            continue;
        };
        let status = cols[2].trim_matches('`').to_string();

        let resolved = resolve_link_target(target);
        let slug = epics
            .iter()
            .find(|e| e.plan.as_deref() == Some(resolved.as_str()))
            .map(|e| e.slug.clone())
            .unwrap_or_else(|| link_stem(target));

        items.push(format!("{slug}={status}"));
    }

    items.sort();
    items
}

/// Canonical `"<slug>=<status>"` items for the registry side, sorted.
fn registry_items(epics: &[Epic]) -> Vec<String> {
    let mut items: Vec<String> = epics
        .iter()
        .map(|e| format!("{}={}", e.slug, canonical_status(&e.status)))
        .collect();
    items.sort();
    items
}

/// Find the HQ brain `epics[]` registry in `ctx.files` — the file whose source is the
/// brain root's own `planning/state.json` and whose `kind == "brain"`.
fn hq_epics(ctx: &ConformanceCtx) -> Option<&Vec<Epic>> {
    let hq_state_path = ctx.root.join("planning").join("state.json");
    ctx.files
        .iter()
        .find(|(src, file)| src.abs_path == hq_state_path && file.kind == "brain")
        .map(|(_, file)| &file.epics)
}

fn not_evaluable(reason: String) -> CheckOutcome {
    CheckOutcome {
        status: CheckStatus::NotEvaluable,
        left: None,
        right: None,
        findings: Vec::new(),
        reason: Some(reason),
    }
}

/// Run the `epics-index-parity` check.
pub fn run(ctx: &ConformanceCtx) -> CheckOutcome {
    let index_path = ctx
        .root
        .join("core")
        .join("planning")
        .join("epics")
        .join("index.md");

    if !index_path.exists() {
        return not_evaluable(format!("index.md not found at {}", index_path.display()));
    }

    let Ok(contents) = std::fs::read_to_string(&index_path) else {
        return not_evaluable(format!(
            "index.md could not be read at {}",
            index_path.display()
        ));
    };

    let Some(epics) = hq_epics(ctx) else {
        return not_evaluable(
            "no HQ brain (kind=\"brain\") planning/state.json loaded for this root".to_string(),
        );
    };

    let left_items = registry_items(epics);
    let right_items = parse_index_md(&contents, epics);

    let left = FactSide {
        label: "state.json epics[]".to_string(),
        source: ctx
            .root
            .join("planning")
            .join("state.json")
            .display()
            .to_string(),
        digest: super::digest(&left_items),
        items: left_items,
    };
    let right = FactSide {
        label: "core/planning/epics/index.md".to_string(),
        source: index_path.display().to_string(),
        digest: super::digest(&right_items),
        items: right_items,
    };

    let mut outcome = compare_sides(left, right);

    // Assert each registry epic's `plan` target exists on disk. A hit here forces the
    // outcome to Drift even when the two item sets otherwise matched.
    let mut missing_doc_findings: Vec<String> = epics
        .iter()
        .filter_map(|e| {
            let plan = e.plan.as_ref()?;
            let doc_path = ctx.root.join(plan);
            if doc_path.exists() {
                None
            } else {
                Some(format!(
                    "missing plan doc for '{}': {} does not exist",
                    e.slug, plan
                ))
            }
        })
        .collect();

    if !missing_doc_findings.is_empty() {
        outcome.status = CheckStatus::Drift;
        outcome.findings.append(&mut missing_doc_findings);
    }

    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain::config::BrainConfig;
    use crate::brain::state::{StateFile, StateSource};

    fn epic(slug: &str, status: &str, plan: Option<&str>) -> Epic {
        Epic {
            slug: slug.to_string(),
            title: slug.to_string(),
            description: None,
            status: Some(status.to_string()),
            weight: None,
            plan: plan.map(|p| p.to_string()),
            repos: Vec::new(),
            extra: serde_json::Map::new(),
        }
    }

    fn state_source(root: &Path) -> StateSource {
        StateSource {
            repo_slug: "hq".to_string(),
            abs_path: root.join("planning").join("state.json"),
            expected_kind: "brain",
        }
    }

    fn state_file(epics: Vec<Epic>) -> StateFile {
        StateFile {
            repo: "hq".to_string(),
            kind: "brain".to_string(),
            updated: "2026-08-03".to_string(),
            focus: Default::default(),
            tracks: Vec::new(),
            repos: Vec::new(),
            cross_repo: Vec::new(),
            tiers: Vec::new(),
            epics,
            note: None,
            backlog: Vec::new(),
            carryover: Vec::new(),
            extra: serde_json::Map::new(),
        }
    }

    fn ctx_with(root: &Path, epics: Vec<Epic>) -> ConformanceCtx {
        ConformanceCtx {
            root: root.to_path_buf(),
            config: BrainConfig::default(),
            files: vec![(state_source(root), state_file(epics))],
        }
    }

    fn write_index_md(root: &Path, body: &str) {
        std::fs::create_dir_all(root.join("core").join("planning").join("epics")).unwrap();
        std::fs::write(
            root.join("core")
                .join("planning")
                .join("epics")
                .join("index.md"),
            body,
        )
        .unwrap();
    }

    fn write_doc(root: &Path, rel: &str) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "stub").unwrap();
    }

    #[test]
    fn matched_sides_pass() {
        let root = crate::testsupport::unique_temp_dir("mev-conformance-epics-match");
        write_index_md(
            &root,
            "| Doc | Epic | Status | Repos |\n|---|---|---|---|\n\
             | [bastion-tui.md](bastion-tui.md) | **Bastion Console** | `paused` | `bastion` |\n",
        );
        write_doc(&root, "core/planning/epics/bastion-tui.md");
        let ctx = ctx_with(
            &root,
            vec![epic(
                "bastion-tui",
                "paused",
                Some("core/planning/epics/bastion-tui.md"),
            )],
        );

        let outcome = run(&ctx);
        assert_eq!(outcome.status, CheckStatus::Pass);
        assert!(outcome.findings.is_empty());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn registry_epic_with_no_index_row_is_drift() {
        let root = crate::testsupport::unique_temp_dir("mev-conformance-epics-missing-row");
        write_index_md(
            &root,
            "| Doc | Epic | Status | Repos |\n|---|---|---|---|\n",
        );
        write_doc(&root, "planning/bullet-proof-software/roadmap.md");
        let ctx = ctx_with(
            &root,
            vec![epic(
                "bullet-proof-software",
                "focused",
                Some("planning/bullet-proof-software/roadmap.md"),
            )],
        );

        let outcome = run(&ctx);
        assert_eq!(outcome.status, CheckStatus::Drift);
        assert!(
            outcome
                .findings
                .iter()
                .any(|f| f.contains("bullet-proof-software"))
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn status_mismatch_on_shared_slug_is_drift_naming_both() {
        let root = crate::testsupport::unique_temp_dir("mev-conformance-epics-status-mismatch");
        write_index_md(
            &root,
            "| Doc | Epic | Status | Repos |\n|---|---|---|---|\n\
             | [brain-engine.md](brain-engine.md) | **Brain Engine** | `complete` | `mev` |\n",
        );
        write_doc(&root, "core/planning/epics/brain-engine.md");
        let ctx = ctx_with(
            &root,
            vec![epic(
                "brain-engine",
                "active",
                Some("core/planning/epics/brain-engine.md"),
            )],
        );

        let outcome = run(&ctx);
        assert_eq!(outcome.status, CheckStatus::Drift);
        let all = outcome.findings.join(" | ");
        assert!(all.contains("brain-engine=active"));
        assert!(all.contains("brain-engine=complete"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_plan_doc_forces_drift_with_finding() {
        let root = crate::testsupport::unique_temp_dir("mev-conformance-epics-missing-doc");
        write_index_md(
            &root,
            "| Doc | Epic | Status | Repos |\n|---|---|---|---|\n\
             | [ghost.md](ghost.md) | **Ghost** | `active` | `mev` |\n",
        );
        // Intentionally do NOT write the doc file — `plan` points nowhere.
        let ctx = ctx_with(
            &root,
            vec![epic(
                "ghost",
                "active",
                Some("core/planning/epics/ghost.md"),
            )],
        );

        let outcome = run(&ctx);
        assert_eq!(outcome.status, CheckStatus::Drift);
        assert!(
            outcome
                .findings
                .iter()
                .any(|f| f.contains("missing plan doc") && f.contains("ghost"))
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn plan_outside_epics_dir_with_correct_relative_link_passes() {
        let root = crate::testsupport::unique_temp_dir("mev-conformance-epics-outside-plan");
        write_index_md(
            &root,
            "| Doc | Epic | Status | Repos |\n|---|---|---|---|\n\
             | [roadmap.md](../../../planning/bullet-proof-software/roadmap.md) | **BPS** | `focused` | `brain` |\n",
        );
        write_doc(&root, "planning/bullet-proof-software/roadmap.md");
        let ctx = ctx_with(
            &root,
            vec![epic(
                "bullet-proof-software",
                "focused",
                Some("planning/bullet-proof-software/roadmap.md"),
            )],
        );

        let outcome = run(&ctx);
        assert_eq!(outcome.status, CheckStatus::Pass, "{:?}", outcome.findings);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_index_md_is_not_evaluable() {
        let root = crate::testsupport::unique_temp_dir("mev-conformance-epics-missing-index");
        std::fs::create_dir_all(&root).unwrap();
        let ctx = ctx_with(&root, vec![epic("x", "active", None)]);

        let outcome = run(&ctx);
        assert_eq!(outcome.status, CheckStatus::NotEvaluable);
        assert!(outcome.reason.is_some());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn resolve_link_target_handles_parent_dirs() {
        assert_eq!(
            resolve_link_target("../../../planning/bullet-proof-software/roadmap.md"),
            "planning/bullet-proof-software/roadmap.md"
        );
        assert_eq!(
            resolve_link_target("bastion-tui.md"),
            "core/planning/epics/bastion-tui.md"
        );
    }

    #[test]
    fn parse_link_target_extracts_the_url() {
        assert_eq!(
            parse_link_target("[bastion-tui.md](bastion-tui.md)"),
            Some("bastion-tui.md")
        );
        assert_eq!(parse_link_target("not a link"), None);
    }
}
