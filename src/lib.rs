//! mev (`mev`) — parses, validates, and (later) compiles the
//! MDX/Markdown content for learn-agentic-ai.com.
//!
//! Phase 0 lays the testable skeleton: a CLI surface and the `Diagnostic` type that every
//! future check emits. Phase 1, Block B adds content-tree crawl + classification (see
//! `planning/master-plan.md`).

pub mod brain;
mod learn_ai;
mod shared;
pub mod theme;
mod validator;
pub use brain::BrainValidator;
pub use brain::crawl::{MdFile, crawl_brain};
pub use brain::emit::{
    EmitAction, EmitError, EmitPlan, apply_plan, plan_master_plan_tables, plan_state_json,
    plan_status_frontmatter, reconcile_status_scalars, render_wave_table, splice_generated,
    wave_order,
};
pub use brain::graph::{Graph, build_graph, check_graph};
pub use brain::graph_emit::{GraphExport, build_graph_export};
pub use brain::links::{
    LinkKind, LinkRef, check_links, check_moved_references, collect_doc_ids, extract_links,
    read_moves_pending,
};
pub use brain::manifest::{Manifest, ManifestEntry, build_manifest};
pub use brain::okf::{OkfFrontmatter, validate_md_file};
pub use brain::visualize::generate_graph_visual;
pub use learn_ai::LearnAiValidator;
pub use learn_ai::crawl::{ContentFile, Corpus, FileKind, Locale, crawl};
pub use learn_ai::meta::validate_file;
pub use validator::ContentValidator;

use std::path::PathBuf;

/// Severity of a single finding. Drives the process exit code: any [`Severity::Error`]
/// makes a run fail (exit 1); warnings are reported but do not fail the run (exit 0).
/// This mirrors the error/warning split of the site's existing `scripts/validate-content.ts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

/// A single validation finding. Every check produces `Diagnostic`s; only the reporter prints.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Diagnostic {
    pub severity: Severity,
    /// File the finding concerns, relative to the content root where possible.
    pub file: PathBuf,
    /// In-file locator (e.g. `metadata.title`, `sections[2].id`) or empty for whole-file findings.
    pub locator: String,
    pub message: String,
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Severity::Error => f.write_str("error"),
            Severity::Warning => f.write_str("warning"),
        }
    }
}

impl Diagnostic {
    pub fn error(
        file: impl Into<PathBuf>,
        locator: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            severity: Severity::Error,
            file: file.into(),
            locator: locator.into(),
            message: message.into(),
        }
    }

    pub fn warning(
        file: impl Into<PathBuf>,
        locator: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            severity: Severity::Warning,
            file: file.into(),
            locator: locator.into(),
            message: message.into(),
        }
    }
}

/// Outcome of a validation run: the findings plus whether they constitute a failure.
#[derive(Debug, Default)]
pub struct Report {
    pub diagnostics: Vec<Diagnostic>,
}

impl Report {
    pub fn error_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .count()
    }

    pub fn warning_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .count()
    }

    /// A run fails when any error-severity diagnostic is present.
    pub fn is_failure(&self) -> bool {
        self.error_count() > 0
    }
}

/// Validate the content tree rooted at `root`.
///
/// Block B: crawl + classify + filename conventions.
/// Block C: struct and frontmatter validation — each file in the [`Corpus`] is dispatched to
/// [`meta::validate_file`], which checks required fields, enum values, and format constraints.
/// All diagnostics (filename + struct/frontmatter) are collected into the returned [`Report`].
///
/// Delegates to [`LearnAiValidator`] via the [`ContentValidator`] trait's default `run` driver.
pub fn validate(root: &std::path::Path) -> anyhow::Result<Report> {
    Ok(LearnAiValidator.run(root))
}

/// Validate the company-brain repo rooted at `root` for OKF frontmatter compliance.
///
/// Phase 2, Block I + Block J-crawl: mirrors [`validate`] for the brain consumer — delegates
/// to [`BrainValidator`] which applies the registry-driven corpus crawl (`crawl_corpus`:
/// `skip_dirs` pruning, corpus membership, scope resolution) and Block H's OKF checks.
/// Root instruction files (`README.md`/`CLAUDE.md`) without frontmatter are exempt from the
/// "missing frontmatter" error — they are included in the corpus as leaves.
///
/// Resolves `brain.toml` by walking up from `root` via [`brain::config::find_brain_config`].
/// If no `brain.toml` is found, a fatal `Error`-severity diagnostic with locator
/// `E_CONFIG_NOT_FOUND` is returned in the report rather than panicking — the caller
/// should treat this as a configuration error (exit 1).
pub fn validate_brain(root: &std::path::Path) -> anyhow::Result<Report> {
    use brain::config::find_brain_config;

    let config = match find_brain_config(root) {
        Ok(cfg) => cfg,
        Err(e) => {
            let mut report = Report::default();
            report.diagnostics.push(Diagnostic::error(
                root,
                "E_CONFIG_NOT_FOUND",
                format!("brain.toml not found or unreadable: {e}"),
            ));
            return Ok(report);
        }
    };

    Ok(BrainValidator::new(config).run(root))
}

/// Validate the company-brain repo for OKF frontmatter compliance **plus** cross-repo
/// sync watermark integrity.
///
/// Phase 3, Block M (HQ-Restructure Block N): runs the full schema pass (identical to
/// [`validate_brain`]) and then appends [`brain::sync::check_sync`] diagnostics into
/// the same [`Report`].  A `Sync` error (any `E_SYNC_*` locator) is `Error`-severity
/// and causes `report.is_failure()` to return `true`, producing exit code 1.
///
/// Resolves `brain.toml` the same way as [`validate_brain`] — see that function's
/// doc for the `E_CONFIG_NOT_FOUND` fallback behaviour.
pub fn validate_brain_sync(root: &std::path::Path) -> anyhow::Result<Report> {
    use brain::config::find_brain_config;
    use brain::sync::check_sync;

    let config = match find_brain_config(root) {
        Ok(cfg) => cfg,
        Err(e) => {
            let mut report = Report::default();
            report.diagnostics.push(Diagnostic::error(
                root,
                "E_CONFIG_NOT_FOUND",
                format!("brain.toml not found or unreadable: {e}"),
            ));
            return Ok(report);
        }
    };

    // Schema pass (OKF frontmatter)
    let mut report = BrainValidator::new(config.clone()).run(root);

    // Sync watermark pass
    let sync_diags = check_sync(root, &config);
    report.diagnostics.extend(sync_diags);

    Ok(report)
}

/// Validate the company-brain repo for OKF frontmatter compliance **plus** the global
/// `scope:doc_id` knowledge-graph integrity check.
///
/// Phase 3, Block J: runs the full schema pass (identical to [`validate_brain`]) and then
/// builds the global graph via [`brain::graph::build_graph`] and appends
/// [`brain::graph::check_graph`] diagnostics into the same [`Report`].
///
/// Graph errors (`E_GRAPH_DUPLICATE_DOC_ID`, `E_GRAPH_DANGLING_RELATED`) cause
/// `report.is_failure()` → `true` (exit 1).  The leaf warning (`W_GRAPH_LEAF_TARGET`)
/// and the isolated-node warning (`W_GRAPH_ISOLATED_NODE`) are reported but do not
/// fail the run on their own.
///
/// Resolves `brain.toml` the same way as [`validate_brain`] — see that function's
/// doc for the `E_CONFIG_NOT_FOUND` fallback behaviour.
pub fn validate_brain_graph(root: &std::path::Path) -> anyhow::Result<Report> {
    use brain::config::find_brain_config;
    use brain::crawl::crawl_corpus;
    use brain::graph::{build_graph, check_graph};

    let config = match find_brain_config(root) {
        Ok(cfg) => cfg,
        Err(e) => {
            let mut report = Report::default();
            report.diagnostics.push(Diagnostic::error(
                root,
                "E_CONFIG_NOT_FOUND",
                format!("brain.toml not found or unreadable: {e}"),
            ));
            return Ok(report);
        }
    };

    // Schema pass (OKF frontmatter) — reuse BrainValidator.
    let mut report = BrainValidator::new(config.clone()).run(root);

    // Graph pass — crawl corpus once, build the graph, then check it.
    let (corpus, crawl_diags) = crawl_corpus(root, &config);
    report.diagnostics.extend(crawl_diags);

    let artifact = build_graph(&corpus, &config);
    let graph_diags = check_graph(&artifact);
    report.diagnostics.extend(graph_diags);

    Ok(report)
}

/// Validate the company-brain repo for OKF frontmatter compliance **plus** the
/// link-integrity pass: flags dead markdown links, dead `file://` URIs, dangling
/// `[[wikilink]]` slugs, and references still pointing at paths listed in
/// `.brain-moves-pending`.
///
/// Phase 3, Block K: runs the full schema pass (identical to [`validate_brain`]) and
/// then crawls the corpus once, calling [`brain::links::check_links`] and
/// [`brain::links::check_moved_references`].  The `doc_ids` set is derived from the
/// same corpus via [`brain::links::collect_doc_ids`] (D5 single-seam discipline).
///
/// Error-severity diagnostics (`E_LINK_*`) cause `report.is_failure()` → `true`
/// (exit 1).
///
/// Resolves `brain.toml` the same way as [`validate_brain`] — see that function's
/// doc for the `E_CONFIG_NOT_FOUND` fallback behaviour.
pub fn validate_brain_links(root: &std::path::Path) -> anyhow::Result<Report> {
    use brain::config::find_brain_config;
    use brain::crawl::crawl_corpus;
    use brain::links::{check_links, check_moved_references, collect_doc_ids, read_moves_pending};

    let config = match find_brain_config(root) {
        Ok(cfg) => cfg,
        Err(e) => {
            let mut report = Report::default();
            report.diagnostics.push(Diagnostic::error(
                root,
                "E_CONFIG_NOT_FOUND",
                format!("brain.toml not found or unreadable: {e}"),
            ));
            return Ok(report);
        }
    };

    // Schema pass (OKF frontmatter) — reuse BrainValidator.
    let mut report = BrainValidator::new(config.clone()).run(root);

    // Link-integrity pass — crawl corpus once, then run both link checks.
    let (corpus, crawl_diags) = crawl_corpus(root, &config);
    report.diagnostics.extend(crawl_diags);

    let doc_ids = collect_doc_ids(&corpus);
    let link_diags = check_links(&corpus, root, &doc_ids);
    report.diagnostics.extend(link_diags);

    let moved_paths = read_moves_pending(root);
    let moved_diags = check_moved_references(&corpus, root, &moved_paths);
    report.diagnostics.extend(moved_diags);

    Ok(report)
}

/// Validate the company-brain repo for OKF frontmatter compliance **plus** the
/// bidirectional `index.md` ↔ directory structural coverage check (D17 / CLAUDE.md
/// Standing Rule 7).
///
/// Phase 3, Block L: runs the full schema pass (identical to [`validate_brain`]) and
/// then crawls the corpus once, calling [`brain::structure::check_structure`] to flag
/// orphan files not listed in their directory's `index.md`
/// (`E_STRUCT_ORPHAN_FILE`) and `index.md` rows pointing at nonexistent targets
/// (`E_STRUCT_DANGLING_ROW`).
///
/// Both diagnostic codes are error-severity, so any finding causes
/// `report.is_failure()` → `true` (exit 1).
///
/// Resolves `brain.toml` the same way as [`validate_brain`] — see that function's
/// doc for the `E_CONFIG_NOT_FOUND` fallback behaviour.
pub fn validate_brain_structure(root: &std::path::Path) -> anyhow::Result<Report> {
    use brain::config::find_brain_config;
    use brain::crawl::crawl_corpus;
    use brain::structure::check_structure;

    let config = match find_brain_config(root) {
        Ok(cfg) => cfg,
        Err(e) => {
            let mut report = Report::default();
            report.diagnostics.push(Diagnostic::error(
                root,
                "E_CONFIG_NOT_FOUND",
                format!("brain.toml not found or unreadable: {e}"),
            ));
            return Ok(report);
        }
    };

    // Schema pass (OKF frontmatter) — reuse BrainValidator.
    let mut report = BrainValidator::new(config.clone()).run(root);

    // Structural coverage pass — crawl corpus once, then check bidirectional coverage.
    let (corpus, crawl_diags) = crawl_corpus(root, &config);
    report.diagnostics.extend(crawl_diags);

    let structure_diags = check_structure(&corpus, root);
    report.diagnostics.extend(structure_diags);

    Ok(report)
}

/// Validate the company-brain repo for OKF frontmatter compliance **plus** the
/// `state.json` schema and cross-repo block-dependency graph integrity checks.
///
/// Phase 3, Block P: runs the full schema pass (identical to [`validate_brain`]) and
/// then appends the state-validation pipeline diagnostics — discovery, schema-ring
/// checks, graph build + integrity checks, and rollup-drift checks — into the same
/// [`Report`].
///
/// Error-severity diagnostics (`E_STATE_*`) cause `report.is_failure()` → `true`
/// (exit 1).  Drift and missing-file warnings (`W_STATE_*`) are reported but do not
/// fail the run on their own (exit 0).
///
/// Resolves `brain.toml` the same way as [`validate_brain`] — see that function's
/// doc for the `E_CONFIG_NOT_FOUND` fallback behaviour.
pub fn validate_brain_state(root: &std::path::Path) -> anyhow::Result<Report> {
    use brain::config::find_brain_config;
    use brain::state::{
        StateLoadError, build_state_graph, check_backlog_integrity, check_backlog_staleness,
        check_carryover_staleness, check_epics, check_field_policy, check_focus_drift,
        check_rollup, check_schema, check_state_graph, check_status_consistency, detect_cycles,
        discover_state_files, load_state,
    };
    use std::collections::HashMap;

    let config = match find_brain_config(root) {
        Ok(cfg) => cfg,
        Err(e) => {
            let mut report = Report::default();
            report.diagnostics.push(Diagnostic::error(
                root,
                "E_CONFIG_NOT_FOUND",
                format!("brain.toml not found or unreadable: {e}"),
            ));
            return Ok(report);
        }
    };

    // Schema pass (OKF frontmatter) — reuse BrainValidator.
    let mut report = BrainValidator::new(config.clone()).run(root);

    // --- State pipeline ---

    // 1. Discovery: find all planning/state.json files.
    let (sources, discovery_diags) = discover_state_files(root, &config);
    report.diagnostics.extend(discovery_diags);

    // 2. Load each discovered file; emit E_STATE_MALFORMED_JSON for parse failures.
    let mut loaded: Vec<(brain::state::StateSource, brain::state::StateFile)> = Vec::new();
    for src in &sources {
        match load_state(&src.abs_path) {
            Ok(file) => {
                // 3. Schema-ring checks on successfully-loaded files.
                let schema_diags = check_schema(src, &file);
                report.diagnostics.extend(schema_diags);
                report.diagnostics.extend(check_field_policy(src, &file));
                loaded.push((src.clone(), file));
            }
            Err(StateLoadError::Parse { source, .. }) => {
                report.diagnostics.push(Diagnostic::error(
                    &src.abs_path,
                    "E_STATE_MALFORMED_JSON",
                    format!(
                        "state.json is not valid JSON or does not match the expected schema: {source}"
                    ),
                ));
            }
            Err(StateLoadError::Io { source, .. }) => {
                // IO errors after discovery are unexpected (file existed during discovery).
                report.diagnostics.push(Diagnostic::error(
                    &src.abs_path,
                    "E_STATE_MALFORMED_JSON",
                    format!("could not read state.json: {source}"),
                ));
            }
        }
    }

    // 4. Graph build + integrity checks.
    let graph = build_state_graph(&loaded);
    let graph_diags = check_state_graph(&graph, &loaded);
    report.diagnostics.extend(graph_diags);

    // 5. Cycle detection — flag any depends_on cycle in the DAG.
    let cycle_diags = detect_cycles(&graph);
    report.diagnostics.extend(cycle_diags);

    // 6. Status-consistency check — closed blocks must not depend on non-closed blocks.
    let consistency_diags = check_status_consistency(&loaded);
    report.diagnostics.extend(consistency_diags);

    // 7. Backlog-node integrity — dangling deps and orphan promoted nodes.
    let backlog_diags = check_backlog_integrity(&loaded, &graph);
    report.diagnostics.extend(backlog_diags);

    // 7b. Epic registry + membership — cross-file, so it runs once over the
    //     whole corpus rather than per-file like check_field_policy.
    report.diagnostics.extend(check_epics(&config, &loaded));

    // 8. Rollup-drift checks (brain files only).
    // Build a slug → StateFile map of all loaded children (project kind).
    let children: HashMap<String, brain::state::StateFile> = loaded
        .iter()
        .filter(|(_, f)| f.kind == "project")
        .map(|(s, f)| (s.repo_slug.clone(), f.clone()))
        .collect();

    for (src, file) in &loaded {
        if file.kind == "brain" {
            let rollup_diags = check_rollup(&src.abs_path, file, &children);
            report.diagnostics.extend(rollup_diags);
        }
    }

    // 9. Focus-drift warnings (per file with tracks[]).
    for (src, file) in &loaded {
        let drift_diags = check_focus_drift(src, file, &config, &graph, &loaded);
        report.diagnostics.extend(drift_diags);
    }

    // 10. Attention staleness warnings — stale carryover (every file) + aging
    //     HQ backlog. WARNING severity only (never flips exit code); malformed
    //     dates are surfaced as E_STATE_DATE_FORMAT by check_schema above.
    let today = chrono::Local::now().date_naive();
    for (src, file) in &loaded {
        report.diagnostics.extend(check_carryover_staleness(
            src,
            file,
            today,
            &config.attention,
        ));
        report
            .diagnostics
            .extend(check_backlog_staleness(src, file, today, &config.attention));
    }

    Ok(report)
}

/// Generate derived views for the company-brain repo and optionally write them.
///
/// Phase 4, Block MV.4.E: the single derivation engine for every generated view the v2
/// state schema declares. Resolves `brain.toml`, discovers and loads all
/// `planning/state.json` files, builds the block-dependency graph, then runs all five
/// planners:
///
/// - [`brain::emit::plan_state_json`] — regenerates leaf `focus` (now/next/blocked)
///   and brain `repos[]`/`cross_repo[]`/`focus` (tier-scoped, non-destructive rollup +
///   repo-tagged focus union) from the authored `tracks[]` DAG.
/// - [`brain::emit::plan_master_plan_tables`] — splices the wave/dependency table into
///   any `master-plan.md` that carries the `<!-- BEGIN generated:wave-table -->`
///   sentinels.
/// - [`brain::emit::plan_project_caches`] — splices the derived focus line + a
///   `synced_from` watermark into each leaf project's `docs/projects/<slug>.md` cache
///   doc (`<!-- BEGIN generated:project-cache -->` sentinels).
/// - [`brain::emit::plan_tier_rollups`] — splices the tier-scoped rollup table into
///   each tier sub-brain's sibling `status.md` (`<!-- BEGIN generated:tier-rollup -->`
///   sentinels).
/// - [`brain::emit::plan_hq_board`] — splices the NOW/NEXT/BLOCKED Operating Board into
///   the HQ brain's `status.md` (`<!-- BEGIN generated:hq-board -->` sentinels).
/// - [`brain::emit::plan_unified_board`] — splices the priority-ranked
///   NOW/NEXT/BLOCKED/DUE-SOON unified board (unioning engineering + business blocks,
///   tagged `[BIZ]`/`[ENG]`) into the HQ brain's `status.md`
///   (`<!-- BEGIN generated:unified-board -->` sentinels; `MV.6.B`).
/// - [`brain::emit::plan_epic_boards`] — splices the per-epic
///   progress/NOW/NEXT/BLOCKED board plus derived cross-epic relationships into every
///   brain-level `status.md` carrying the `<!-- BEGIN generated:epic-board -->`
///   sentinels. Lanes are global (an epic is cross-repo by definition); only *which*
///   epics appear is tier-scoped.
/// - [`brain::emit::plan_epic_sequences`] — splices each epic's cross-repo wave-ordered
///   sequence table into the `plan` doc its registry entry points at
///   (`<!-- BEGIN generated:epic-sequence -->` sentinels).
///
/// When `write` is `false` (default), the function is a **dry-run**: no files are
/// written and each planned action is reported as a `W_EMIT_DRY_RUN` diagnostic.
/// When `write` is `true`, the derived content is written in place and each write
/// is reported as an `I_EMIT_WROTE` diagnostic.
///
/// A target file lacking the relevant sentinels is skipped with a `W_EMIT_NO_SENTINEL`
/// warning — the emitter never splices into arbitrary prose.
///
/// **Corpus-completeness guard.** When `write` is `true` and any discovered
/// `state.json` failed to load (an `E_STATE_MALFORMED_JSON` diagnostic was pushed),
/// `emit_state` refuses to regenerate any derived surface: it pushes an
/// `E_EMIT_INCOMPLETE_CORPUS` error diagnostic and returns *before*
/// `build_state_graph` and the first planner runs, leaving every file untouched.
/// Derived views are cross-repo unions, so writing them from a partial corpus would
/// silently erase the missing repo(s) — and rewriting `cross_repo[]` would delete the
/// dangling references that are the only evidence. Dry-run (`write == false`) is
/// entirely exempt from this guard: it still runs every planner and reports
/// `W_EMIT_DRY_RUN` actions, alongside the `E_STATE_MALFORMED_JSON` cause, so it
/// remains the diagnostic tool for exactly this situation.
///
/// Resolves `brain.toml` the same way as [`validate_brain`] — see that function's
/// doc for the `E_CONFIG_NOT_FOUND` fallback behaviour.
/// Park or un-park an epic, cascading to its member blocks (`mev defer-epic` /
/// `mev resume-epic` / `mev sync-epics`).
///
/// `action` selects the direction; `slug` names one epic, or `None` reconciles the
/// whole registry in both directions (see [`brain::epics::plan_sync_epics`]).
///
/// Dry-run by default, exactly like [`emit_state`]: without `write` the proposed
/// authored edits are reported as `W_EMIT_DRY_RUN` and nothing is touched. With
/// `write`, the authored edits are applied **and then [`emit_state`] is run** so
/// the derived views (`focus`, boards, rollups) are regenerated in the same
/// invocation — otherwise the corpus would be left in a state the drift checks
/// immediately complain about.
///
/// This is the one place mev writes *authored* state; see the module docs on
/// `brain::epics` for why that lives behind an explicit command rather than in
/// `emit-state`.
pub fn epic_status(
    root: &std::path::Path,
    slug: Option<&str>,
    action: brain::epics::EpicAction,
    write: bool,
) -> anyhow::Result<Report> {
    use brain::config::find_brain_config;
    use brain::emit::apply_plan;
    use brain::epics::{EpicAction, plan_defer_epic, plan_resume_epic, plan_sync_epics};
    use brain::state::{StateLoadError, discover_state_files, load_state};

    let mut report = Report::default();

    let config = match find_brain_config(root) {
        Ok(cfg) => cfg,
        Err(e) => {
            report.diagnostics.push(Diagnostic::error(
                root,
                "E_CONFIG_NOT_FOUND",
                format!("brain.toml not found or unreadable: {e}"),
            ));
            return Ok(report);
        }
    };

    let (sources, discovery_diags) = discover_state_files(root, &config);
    report.diagnostics.extend(discovery_diags);

    let mut loaded: Vec<(brain::state::StateSource, brain::state::StateFile)> = Vec::new();
    let mut load_failed = false;
    for src in &sources {
        match load_state(&src.abs_path) {
            Ok(file) => loaded.push((src.clone(), file)),
            Err(StateLoadError::Parse { source, .. }) => {
                load_failed = true;
                report.diagnostics.push(Diagnostic::error(
                    &src.abs_path,
                    "E_STATE_MALFORMED_JSON",
                    format!("state.json is not valid JSON or does not match the schema: {source}"),
                ));
            }
            Err(StateLoadError::Io { source, .. }) => {
                load_failed = true;
                report.diagnostics.push(Diagnostic::error(
                    &src.abs_path,
                    "E_STATE_MALFORMED_JSON",
                    format!("could not read state.json: {source}"),
                ));
            }
        }
    }

    // Same completeness guard as emit-state: an epic's members span repos, so
    // cascading from a partial corpus would silently skip whole repos' blocks.
    if write && load_failed {
        report.diagnostics.push(Diagnostic::error(
            root,
            "E_EPIC_INCOMPLETE_CORPUS",
            "refusing to write: at least one state.json failed to load, and an epic's members \
             span repos — cascading now would silently skip the unreadable repo's blocks"
                .to_string(),
        ));
        return Ok(report);
    }

    let plan = match (slug, action) {
        (Some(s), EpicAction::Defer) => plan_defer_epic(s, &config, &loaded),
        (Some(s), EpicAction::Resume) => plan_resume_epic(s, &config, &loaded),
        (None, _) => plan_sync_epics(&config, &loaded),
    };

    let had_actions = !plan.actions.is_empty();
    report.diagnostics.extend(apply_plan(&plan, write));

    // Regenerate derived views so focus/boards agree with the authored edit.
    if write && had_actions && !report.is_failure() {
        let emit = emit_state(root, true)?;
        report.diagnostics.extend(emit.diagnostics);
    }

    Ok(report)
}

pub fn emit_state(root: &std::path::Path, write: bool) -> anyhow::Result<Report> {
    use brain::config::find_brain_config;
    use brain::emit::{
        apply_plan, plan_attention_board, plan_brain_cache_watermarks, plan_epic_boards,
        plan_epic_sequences, plan_hq_board, plan_master_plan_tables, plan_project_caches,
        plan_state_json, plan_status_frontmatter, plan_tier_rollups, plan_unified_board,
    };
    use brain::state::{StateLoadError, build_state_graph, discover_state_files, load_state};

    let config = match find_brain_config(root) {
        Ok(cfg) => cfg,
        Err(e) => {
            let mut report = Report::default();
            report.diagnostics.push(Diagnostic::error(
                root,
                "E_CONFIG_NOT_FOUND",
                format!("brain.toml not found or unreadable: {e}"),
            ));
            return Ok(report);
        }
    };

    let mut report = Report::default();

    // 1. Discovery: find all planning/state.json files.
    let (sources, discovery_diags) = discover_state_files(root, &config);
    report.diagnostics.extend(discovery_diags);

    // 2. Load each discovered file.
    let mut loaded: Vec<(brain::state::StateSource, brain::state::StateFile)> = Vec::new();
    for src in &sources {
        match load_state(&src.abs_path) {
            Ok(file) => {
                loaded.push((src.clone(), file));
            }
            Err(StateLoadError::Parse { source, .. }) => {
                report.diagnostics.push(Diagnostic::error(
                    &src.abs_path,
                    "E_STATE_MALFORMED_JSON",
                    format!(
                        "state.json is not valid JSON or does not match the expected schema: {source}"
                    ),
                ));
            }
            Err(StateLoadError::Io { source, .. }) => {
                report.diagnostics.push(Diagnostic::error(
                    &src.abs_path,
                    "E_STATE_MALFORMED_JSON",
                    format!("could not read state.json: {source}"),
                ));
            }
        }
    }

    // 2.5. Completeness guard: refuse to write derived views from a partial corpus.
    //    Derived views (focus, repos[]/cross_repo[], project caches, tier rollups,
    //    HQ/unified/epic boards, master-plan and epic sequence tables) are all
    //    cross-repo unions — regenerating them while any discovered state.json
    //    failed to load would silently erase the missing repo(s) from every
    //    surface, and rewriting cross_repo[] would delete the dangling references
    //    that are the only evidence a repo went missing. This must run before
    //    `build_state_graph`/the first `apply_plan` and must NOT affect dry-run —
    //    the `write &&` conjunct is load-bearing and must stay first.
    if write && loaded.len() < sources.len() {
        report.diagnostics.push(Diagnostic::error(
            root,
            "E_EMIT_INCOMPLETE_CORPUS",
            format!(
                "refusing to write derived views: {} of {} discovered state.json files failed to load. Derived views (focus, repos[]/cross_repo[], project caches, tier rollups, HQ/unified/epic boards, master-plan and epic sequence tables) are cross-repo unions — regenerating them from a partial corpus would silently erase the missing repo(s) from every surface, and rewriting cross_repo[] would delete the dangling references that are the only evidence. Fix the E_STATE_MALFORMED_JSON error(s) above, then re-run.",
                sources.len() - loaded.len(),
                sources.len()
            ),
        ));
        return Ok(report);
    }

    // 3. Build the block-dependency graph.
    let graph = build_state_graph(&loaded);

    // 4-5. Run each planner immediately followed by its own apply, in a stable
    //    order — rather than planning all eight up front and applying them as
    //    a batch afterward. Every planner that splices a rendered section
    //    re-reads its target document at call time; several targets are
    //    shared (status.md carries HQ_BOARD + UNIFIED_BOARD + ATTENTION for
    //    the HQ root, and TIER_ROLLUP + ATTENTION for a tier sub-brain). If
    //    all eight were planned before any of them wrote, every planner would
    //    see the same pre-batch original and each write would silently drop
    //    the previous one's sentinel edit (the emit-state-same-file-batching
    //    known issue). Interleaving plan+apply means a later planner targeting
    //    the same file reads the up-to-date text left by an earlier one.
    //    `loaded`/`graph` stay the single fixed snapshot computed above for
    //    every planner — only the on-disk *rendered documents* progress as we
    //    go, not the corpus data driving what gets rendered. Order still
    //    matters for shared targets: attention lands after hq_board/
    //    tier_rollups/unified_board (its sibling sentinels in the same
    //    files), and plan_status_frontmatter (below) runs last of all so it
    //    reads the fully updated text in write mode.
    let state_plan = plan_state_json(&loaded, &graph, &config);
    let state_diags = apply_plan(&state_plan, write);

    let mp_plan = plan_master_plan_tables(&loaded, &graph);
    let mp_diags = apply_plan(&mp_plan, write);

    let project_caches_plan = plan_project_caches(root, &loaded, &graph, &config);
    let project_caches_diags = apply_plan(&project_caches_plan, write);

    let tier_rollups_plan = plan_tier_rollups(&loaded, &graph, &config);
    let tier_rollups_diags = apply_plan(&tier_rollups_plan, write);

    let hq_board_plan = plan_hq_board(&loaded, &graph, &config);
    let hq_board_diags = apply_plan(&hq_board_plan, write);

    let today = chrono::Local::now().date_naive();
    let unified_board_plan = plan_unified_board(&loaded, &graph, &config, today);
    let unified_board_diags = apply_plan(&unified_board_plan, write);

    let attention_plan = plan_attention_board(&loaded, &config, today);
    let attention_diags = apply_plan(&attention_plan, write);

    let brain_caches_plan = plan_brain_cache_watermarks(root, &loaded, &config);
    let brain_caches_diags = apply_plan(&brain_caches_plan, write);

    // 6. Epic boards + sequence tables run after every planner above has both
    //    planned and applied. `plan_epic_boards` shares `status.md` with the
    //    HQ/unified/attention boards, so it reads their already-applied text;
    //    `plan_epic_sequences` targets its own `epics/<slug>.md` docs, which
    //    nothing above touches. The two are still planned together and applied
    //    together (not interleaved) — safe, since they target disjoint files.
    let epic_board_plan = plan_epic_boards(&loaded, &graph, &config);
    let epic_seq_plan = plan_epic_sequences(root, &loaded, &graph, &config);
    let epic_board_diags = apply_plan(&epic_board_plan, write);
    let epic_seq_diags = apply_plan(&epic_seq_plan, write);

    // 7. Run and apply the YAML frontmatter planner last so it sees the updated markdown in write mode.
    let status_fm_plan = plan_status_frontmatter(root, &loaded, &graph, &config);
    let status_fm_diags = apply_plan(&status_fm_plan, write);

    report.diagnostics.extend(state_diags);
    report.diagnostics.extend(mp_diags);
    report.diagnostics.extend(project_caches_diags);
    report.diagnostics.extend(tier_rollups_diags);
    report.diagnostics.extend(hq_board_diags);
    report.diagnostics.extend(unified_board_diags);
    report.diagnostics.extend(attention_diags);
    report.diagnostics.extend(brain_caches_diags);
    report.diagnostics.extend(epic_board_diags);
    report.diagnostics.extend(epic_seq_diags);
    report.diagnostics.extend(status_fm_diags);

    Ok(report)
}

/// Crawl the company-brain repo rooted at `root` and emit a canonical JSON manifest.
///
/// Phase 3, Block Q: resolves `brain.toml` by walking up from `root`, calls
/// [`brain::crawl::crawl_corpus`] once to obtain the full file list, and passes the
/// result to [`brain::manifest::build_manifest`] to produce a [`Manifest`] value.
///
/// The returned [`Manifest`] is a pure value — nothing is written to disk.  The caller
/// serialises it to stdout (or a file) as needed (consistent with the D4 pure-compiler model).
///
/// Returns an [`anyhow::Error`] only for hard configuration errors (e.g. `brain.toml` not
/// found).  Crawl diagnostics are discarded here — the combined validate-and-manifest flow
/// should use `validate_brain` first if diagnostic reporting is required.
pub fn manifest_brain(root: &std::path::Path) -> anyhow::Result<Manifest> {
    use brain::config::find_brain_config;
    use brain::crawl::crawl_corpus;

    let config = find_brain_config(root)
        .map_err(|e| anyhow::anyhow!("brain.toml not found or unreadable: {e}"))?;

    let (corpus, _crawl_diags) = crawl_corpus(root, &config);
    Ok(build_manifest(root, &corpus))
}

/// Build the graph-export envelope for a Brain corpus crawl.
///
/// Discovers `brain.toml`, crawls the corpus, builds the `scope:doc_id` knowledge graph, and
/// converts it into a [`GraphExport`] envelope. Mirrors [`manifest_brain`]; this is a pure
/// compiler — nothing is written to disk or a DB (D4). The caller (the `emit-graph` CLI
/// subcommand) serialises the result to stdout.
pub fn graph_brain(root: &std::path::Path) -> anyhow::Result<GraphExport> {
    use brain::config::find_brain_config;
    use brain::crawl::crawl_corpus;

    let config = find_brain_config(root)
        .map_err(|e| anyhow::anyhow!("brain.toml not found or unreadable: {e}"))?;

    let (corpus, _crawl_diags) = crawl_corpus(root, &config);
    let artifact = build_graph(&corpus, &config);
    Ok(build_graph_export(root, &artifact))
}

/// Generate an interactive HTML knowledge graph of the Brain corpus.
///
/// Discovers `brain.toml`, runs a crawl to build a manifest, and delegates to
/// `generate_graph_visual` to write `graph.md` and `graph.html` to `out_dir`.
/// If `out_dir` is `None`, defaults to `planning/doc-graph` under the brain root.
pub fn visualize_brain(root: &std::path::Path, out_dir: Option<PathBuf>) -> anyhow::Result<()> {
    let out = out_dir.unwrap_or_else(|| root.join("planning").join("doc-graph"));

    let manifest = manifest_brain(root)?;
    brain::visualize::generate_graph_visual(&manifest, &out)
}

/// Machine-readable envelope emitted by the `--json` flag for any `mev` subcommand.
///
/// Consumed by the Brain RAG indexer as a pre-`--rebuild` gate.
#[derive(Debug, serde::Serialize)]
pub struct JsonReport {
    /// Which validator produced this report (`"brain"` or `"learn-ai"`).
    pub validator: String,
    /// Display path of the root that was validated.
    pub root: String,
    /// Number of error-severity diagnostics.
    pub errors: usize,
    /// Number of warning-severity diagnostics.
    pub warnings: usize,
    /// All diagnostics emitted during the run.
    pub diagnostics: Vec<Diagnostic>,
}

impl JsonReport {
    /// Build a [`JsonReport`] from the component pieces.
    pub fn new(validator: &str, root: &std::path::Path, report: &Report) -> Self {
        Self {
            validator: validator.to_owned(),
            root: root.display().to_string(),
            errors: report.error_count(),
            warnings: report.warning_count(),
            diagnostics: report.diagnostics.clone(),
        }
    }

    /// Serialize to a pretty-printed JSON string.
    pub fn to_json(&self) -> anyhow::Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }
}
