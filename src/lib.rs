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
        StateLoadError, build_state_graph, check_backlog_integrity, check_field_policy,
        check_focus_drift, check_rollup, check_schema, check_state_graph, check_status_consistency,
        detect_cycles, discover_state_files, load_state,
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
///
/// When `write` is `false` (default), the function is a **dry-run**: no files are
/// written and each planned action is reported as a `W_EMIT_DRY_RUN` diagnostic.
/// When `write` is `true`, the derived content is written in place and each write
/// is reported as an `I_EMIT_WROTE` diagnostic.
///
/// A target file lacking the relevant sentinels is skipped with a `W_EMIT_NO_SENTINEL`
/// warning — the emitter never splices into arbitrary prose.
///
/// Resolves `brain.toml` the same way as [`validate_brain`] — see that function's
/// doc for the `E_CONFIG_NOT_FOUND` fallback behaviour.
pub fn emit_state(root: &std::path::Path, write: bool) -> anyhow::Result<Report> {
    use brain::config::find_brain_config;
    use brain::emit::{
        apply_plan, plan_brain_cache_watermarks, plan_hq_board, plan_master_plan_tables,
        plan_project_caches, plan_state_json, plan_status_frontmatter, plan_tier_rollups,
        plan_unified_board,
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

    // 3. Build the block-dependency graph.
    let graph = build_state_graph(&loaded);

    // 4. Run all five planners.
    let state_plan = plan_state_json(&loaded, &graph, &config);
    let mp_plan = plan_master_plan_tables(&loaded, &graph);
    let project_caches_plan = plan_project_caches(root, &loaded, &graph, &config);
    let tier_rollups_plan = plan_tier_rollups(&loaded, &graph, &config);
    let hq_board_plan = plan_hq_board(&loaded, &graph, &config);
    let unified_board_plan =
        plan_unified_board(&loaded, &graph, &config, chrono::Local::now().date_naive());
    let brain_caches_plan = plan_brain_cache_watermarks(root, &loaded, &config);

    // 5. Apply all plans (write or dry-run), in a stable order.
    let state_diags = apply_plan(&state_plan, write);
    let mp_diags = apply_plan(&mp_plan, write);
    let project_caches_diags = apply_plan(&project_caches_plan, write);
    let tier_rollups_diags = apply_plan(&tier_rollups_plan, write);
    let hq_board_diags = apply_plan(&hq_board_plan, write);
    let unified_board_diags = apply_plan(&unified_board_plan, write);
    let brain_caches_diags = apply_plan(&brain_caches_plan, write);

    // 6. Run and apply the YAML frontmatter planner last so it sees the updated markdown in write mode.
    let status_fm_plan = plan_status_frontmatter(root, &loaded, &graph, &config);
    let status_fm_diags = apply_plan(&status_fm_plan, write);

    report.diagnostics.extend(state_diags);
    report.diagnostics.extend(mp_diags);
    report.diagnostics.extend(project_caches_diags);
    report.diagnostics.extend(tier_rollups_diags);
    report.diagnostics.extend(hq_board_diags);
    report.diagnostics.extend(unified_board_diags);
    report.diagnostics.extend(brain_caches_diags);
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
