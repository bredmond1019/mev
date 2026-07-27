//! Idempotent `index.md` table-row upsert from an `okf_core::IndexIntent`
//! (`MV.9.A` task 2).
//!
//! This is the second half of the generic materializer, alongside
//! `materialize::plan_document` (the document write itself). Together they
//! let one `plan_document` call plan both the document write and its
//! `index.md` row via `EmitPlan::extend`.
//!
//! Like `materialize::plan_document`, this module is a **planner**: the one
//! read of the existing `index.md` happens during planning so
//! `crate::brain::emit::apply_plan` stays the single write point.

use std::path::Path;

use okf_core::IndexIntent;

use crate::Diagnostic;
use crate::brain::emit::{EmitAction, EmitPlan};

/// A located markdown table within a file's lines: the header row index, the
/// column count (derived from the header), and the inclusive range of body
/// row line indices (may be empty).
struct Table {
    header_idx: usize,
    column_count: usize,
    /// Indices (into the file's lines) of the body rows, in order.
    body_row_indices: Vec<usize>,
}

/// True when `line` is a markdown table separator row (e.g. `|---|---|` or
/// `| :-- | --: |`) — only `|`, `-`, `:`, and whitespace, with at least one
/// `-`.
fn is_separator_line(line: &str) -> bool {
    let t = line.trim();
    if t.is_empty() || !t.contains('-') {
        return false;
    }
    t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '))
}

/// True when `line` looks like a markdown table row: contains at least one
/// `|` and is not blank.
fn is_row_line(line: &str) -> bool {
    let t = line.trim();
    !t.is_empty() && t.contains('|')
}

/// Split a markdown table row into its trimmed cell strings, tolerating an
/// optional leading/trailing `|`.
fn split_row(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    t.split('|').map(|c| c.trim().to_string()).collect()
}

/// Render a row from cell strings, matching the `| a | b | c |` style used
/// throughout the brain's index tables.
fn render_row(cells: &[String]) -> String {
    format!("| {} |", cells.join(" | "))
}

/// Locate the first markdown table in `lines`: a header row immediately
/// followed by a separator row, then zero or more contiguous row lines.
fn find_first_table(lines: &[&str]) -> Option<Table> {
    for i in 0..lines.len().saturating_sub(1) {
        if is_row_line(lines[i]) && is_separator_line(lines[i + 1]) {
            let column_count = split_row(lines[i]).len();
            let mut body_row_indices = Vec::new();
            let mut j = i + 2;
            while j < lines.len() && is_row_line(lines[j]) {
                body_row_indices.push(j);
                j += 1;
            }
            return Some(Table {
                header_idx: i,
                column_count,
                body_row_indices,
            });
        }
    }
    None
}

/// True when a table row's raw first cell contains a markdown link to
/// `link_target`, e.g. `[Anthropic](anthropic.md)` links to `anthropic.md`.
fn row_links_to(first_cell: &str, link_target: &str) -> bool {
    first_cell.contains(&format!("]({link_target})"))
}

/// Plan the upsert (if any) of `intent`'s row into its target `index.md`
/// under `root`.
///
/// - Missing `index.md` → `W_DOC_INDEX_MISSING`, no action (never created).
/// - No table found in the file → `W_DOC_INDEX_NO_TABLE`, no action.
/// - `intent.row_cells.len()` not matching the table's column count →
///   `W_DOC_INDEX_COLUMN_MISMATCH`, no action.
/// - A body row whose first cell links to `intent.link_target` is replaced
///   in place; otherwise one new row is appended at the end of the table.
///   No other row is reordered or altered, and no byte outside the table is
///   touched.
/// - When the upsert produces byte-identical content to the existing file,
///   no action is planned and `W_DOC_UNCHANGED` is pushed instead.
pub fn plan_index_reconcile(intent: &IndexIntent, root: &Path) -> EmitPlan {
    let path = root.join(&intent.index_path);

    let Ok(existing) = std::fs::read_to_string(&path) else {
        return EmitPlan {
            actions: vec![],
            diagnostics: vec![Diagnostic::warning(
                &path,
                "W_DOC_INDEX_MISSING",
                format!("index file {} not found", path.display()),
            )],
        };
    };

    let trailing_newline = existing.ends_with('\n');
    let lines: Vec<&str> = existing.lines().collect();

    let Some(table) = find_first_table(&lines) else {
        return EmitPlan {
            actions: vec![],
            diagnostics: vec![Diagnostic::warning(
                &path,
                "W_DOC_INDEX_NO_TABLE",
                format!("no markdown table found in {}", path.display()),
            )],
        };
    };

    if intent.row_cells.len() != table.column_count {
        return EmitPlan {
            actions: vec![],
            diagnostics: vec![Diagnostic::warning(
                &path,
                "W_DOC_INDEX_COLUMN_MISMATCH",
                format!(
                    "row_cells has {} cell(s) but the table at {} has {} column(s)",
                    intent.row_cells.len(),
                    path.display(),
                    table.column_count
                ),
            )],
        };
    }

    let mut new_cells = intent.row_cells.clone();
    new_cells[0] = format!("[{}]({})", new_cells[0], intent.link_target);
    let new_row = render_row(&new_cells);

    let existing_match = table.body_row_indices.iter().find(|&&idx| {
        let cells = split_row(lines[idx]);
        cells
            .first()
            .is_some_and(|c0| row_links_to(c0, &intent.link_target))
    });

    let mut new_lines: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    match existing_match {
        Some(&idx) => {
            new_lines[idx] = new_row;
        }
        None => {
            let insert_at = table
                .body_row_indices
                .last()
                .map(|&idx| idx + 1)
                .unwrap_or(table.header_idx + 2);
            new_lines.insert(insert_at, new_row);
        }
    }

    let mut new_content = new_lines.join("\n");
    if trailing_newline {
        new_content.push('\n');
    }

    if new_content == existing {
        return EmitPlan {
            actions: vec![],
            diagnostics: vec![Diagnostic::warning(
                &path,
                "W_DOC_UNCHANGED",
                format!("{} index row is already up to date", path.display()),
            )],
        };
    }

    EmitPlan {
        actions: vec![EmitAction {
            path: path.clone(),
            new_content,
            note: format!(
                "{} index row for '{}' in {}",
                if existing_match.is_some() {
                    "update"
                } else {
                    "insert"
                },
                intent.link_target,
                path.display()
            ),
        }],
        diagnostics: vec![],
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_index(dir: &Path, contents: &str) -> std::path::PathBuf {
        let path = dir.join("index.md");
        fs::write(&path, contents).unwrap();
        path
    }

    const SAMPLE: &str = "# Opportunities\n\n## Files\n\n| Opportunity | Kind | Stage |\n|---|---|---|\n| [Anthropic](anthropic.md) | `company` | `identified` |\n";

    #[test]
    fn inserts_row_when_absent() {
        let tmp = tempfile::tempdir().unwrap();
        write_index(tmp.path(), SAMPLE);

        let intent = IndexIntent::new(
            "index.md",
            "acme.md",
            vec![
                "Acme".to_string(),
                "`company`".to_string(),
                "`identified`".to_string(),
            ],
        );

        let plan = plan_index_reconcile(&intent, tmp.path());
        assert_eq!(plan.actions.len(), 1);
        assert!(plan.diagnostics.is_empty());
        let content = &plan.actions[0].new_content;
        assert!(content.contains("[Acme](acme.md)"));
        // Existing row untouched.
        assert!(content.contains("[Anthropic](anthropic.md)"));
        // New row appended after the existing one.
        let anthropic_pos = content.find("Anthropic").unwrap();
        let acme_pos = content.find("Acme").unwrap();
        assert!(anthropic_pos < acme_pos);
    }

    #[test]
    fn updates_row_in_place_when_present() {
        let tmp = tempfile::tempdir().unwrap();
        write_index(tmp.path(), SAMPLE);

        let intent = IndexIntent::new(
            "index.md",
            "anthropic.md",
            vec![
                "Anthropic".to_string(),
                "`company`".to_string(),
                "`contacted`".to_string(),
            ],
        );

        let plan = plan_index_reconcile(&intent, tmp.path());
        assert_eq!(plan.actions.len(), 1);
        let content = &plan.actions[0].new_content;
        assert!(content.contains("| [Anthropic](anthropic.md) | `company` | `contacted` |"));
        // Row count unchanged: exactly one occurrence of "Anthropic".
        assert_eq!(content.matches("Anthropic").count(), 1);
    }

    #[test]
    fn double_run_plans_zero_actions() {
        let tmp = tempfile::tempdir().unwrap();
        write_index(tmp.path(), SAMPLE);

        let intent = IndexIntent::new(
            "index.md",
            "anthropic.md",
            vec![
                "Anthropic".to_string(),
                "`company`".to_string(),
                "`identified`".to_string(),
            ],
        );

        let plan = plan_index_reconcile(&intent, tmp.path());
        assert!(plan.actions.is_empty());
        assert_eq!(plan.diagnostics.len(), 1);
        assert_eq!(plan.diagnostics[0].locator, "W_DOC_UNCHANGED");
    }

    #[test]
    fn missing_index_yields_warning_and_no_action() {
        let tmp = tempfile::tempdir().unwrap();
        let intent = IndexIntent::new("index.md", "acme.md", vec!["Acme".to_string()]);

        let plan = plan_index_reconcile(&intent, tmp.path());
        assert!(plan.actions.is_empty());
        assert_eq!(plan.diagnostics.len(), 1);
        assert_eq!(plan.diagnostics[0].locator, "W_DOC_INDEX_MISSING");
    }

    #[test]
    fn missing_table_yields_warning_and_no_action() {
        let tmp = tempfile::tempdir().unwrap();
        write_index(tmp.path(), "# No table here\n\nJust prose.\n");
        let intent = IndexIntent::new("index.md", "acme.md", vec!["Acme".to_string()]);

        let plan = plan_index_reconcile(&intent, tmp.path());
        assert!(plan.actions.is_empty());
        assert_eq!(plan.diagnostics.len(), 1);
        assert_eq!(plan.diagnostics[0].locator, "W_DOC_INDEX_NO_TABLE");
    }

    #[test]
    fn column_mismatch_yields_warning_and_no_action() {
        let tmp = tempfile::tempdir().unwrap();
        write_index(tmp.path(), SAMPLE);
        let intent = IndexIntent::new(
            "index.md",
            "acme.md",
            vec!["Acme".to_string(), "`company`".to_string()],
        );

        let plan = plan_index_reconcile(&intent, tmp.path());
        assert!(plan.actions.is_empty());
        assert_eq!(plan.diagnostics.len(), 1);
        assert_eq!(plan.diagnostics[0].locator, "W_DOC_INDEX_COLUMN_MISMATCH");
    }
}
