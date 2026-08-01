//! `mev` CLI entry point. Thin wrapper over the library: parse args, dispatch, set exit code.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use mev::Severity;
use mev::theme;

/// `--scope` mode for `mev emit-block-graph`. Maps onto
/// [`mev::brain::block_graph::BlockGraphScope`]'s `tier`/`epic`/`repo` fields.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum BlockGraphScopeArg {
    /// Every repo (`TierScope::All`).
    Hq,
    /// Repos in `--tier`.
    Tier,
    /// `--repo <SLUG>` intersected against the full corpus.
    Repo,
    /// `--epic <SLUG>` projection; overrides `--tier`/`--repo`.
    Epic,
}

#[derive(Parser)]
#[command(
    name = "mev",
    version,
    about = "Validate Markdown/MDX content: learn-agentic-ai.com content and Bastion Brain OKF frontmatter"
)]
struct Cli {
    /// Emit machine-readable JSON envelope to stdout instead of a human summary.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Validate the learn-ai content tree (Phase 1: learn modules).
    Validate {
        /// Path to the content root (e.g. ../learn-ai/content/learn).
        #[arg(default_value = "../learn-ai/content/learn")]
        path: PathBuf,
    },
    /// Validate the Bastion Brain repo for OKF frontmatter compliance (Phase 2).
    /// With --sync, also checks cross-repo synced_from watermark integrity (Phase 3, Block M).
    /// With --graph, also checks the global scope:doc_id knowledge-graph integrity (Phase 3, Block J).
    /// With --state, also checks each repo's planning/state.json schema and cross-repo block graph (Phase 3, Block P).
    /// With --links, also checks markdown, file://, and [[wikilink]] references for dead or moved targets (Phase 3, Block K).
    /// With --structure, also checks bidirectional index.md <-> directory coverage: orphan files and dangling rows (Phase 3, Block L).
    ValidateBrain {
        /// Path to search from when locating brain.toml (walks up to find it).
        /// Defaults to the current directory.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Also run the cross-repo sync watermark check: compares each sub-repo's
        /// planning/status.md `timestamp` against its brain cache doc's `synced_from`.
        /// A mismatch emits an E_SYNC_DRIFT error (exit 1).
        #[arg(long)]
        sync: bool,
        /// Also run the global scope:doc_id knowledge-graph integrity check: flags duplicate
        /// canonical ids (E_GRAPH_DUPLICATE_DOC_ID), dangling related: edges
        /// (E_GRAPH_DANGLING_RELATED), related: entries pointing at leaf files
        /// (W_GRAPH_LEAF_TARGET), and nodes with zero outbound related: edges
        /// (W_GRAPH_ISOLATED_NODE). Graph errors cause exit 1; the warnings alone do not.
        #[arg(long)]
        graph: bool,
        /// Also run the state.json schema and block-dependency graph integrity check: validates
        /// each repo's planning/state.json against the canonical schema, checks for dangling
        /// blocked_by references (E_STATE_DANGLING_BLOCKED_BY), unknown repos
        /// (E_STATE_UNKNOWN_REPO), and flags rollup drift (W_STATE_ROLLUP_DRIFT).
        /// State errors cause exit 1; drift-only warnings exit 0.
        #[arg(long)]
        state: bool,
        /// Also run the link-integrity pass: flags dead markdown [text](path) links
        /// (E_LINK_DEAD_MARKDOWN), dead file:// URIs (E_LINK_DEAD_FILE_URI), dangling
        /// [[wikilink]] slugs (E_LINK_DANGLING_WIKILINK), and references still pointing
        /// at paths listed in .brain-moves-pending (E_LINK_MOVED_REFERENCE).
        /// Link errors cause exit 1.
        #[arg(long)]
        links: bool,
        /// Also run the bidirectional index.md <-> directory structural coverage check:
        /// flags a corpus file not referenced by its directory's index.md
        /// (E_STRUCT_ORPHAN_FILE), and an index.md row (markdown or file:// link) pointing
        /// at a nonexistent target (E_STRUCT_DANGLING_ROW). Structural errors cause exit 1.
        /// Dispatch precedence: --links takes priority over --structure, which takes
        /// priority over --state, --graph, and --sync (checked in that order; first
        /// matching flag wins; no flags falls back to the base OKF schema pass).
        #[arg(long)]
        structure: bool,
    },
    /// Emit a JSON manifest of every file in the Brain corpus (Phase 3, Block Q).
    ///
    /// Crawls the Brain repo, resolves `brain.toml`, and prints a JSON document listing
    /// every corpus file with its scope, relative path, and OKF metadata fields.  The
    /// manifest is the single source of truth for `index_brain.py` — "what's validated ==
    /// what's embedded" holds by construction.
    ///
    /// Output is compact JSON by default; pass --pretty for indented output.
    ///
    /// Exit codes:
    ///   0 — manifest emitted successfully
    ///   1 — configuration error (brain.toml not found or unreadable)
    Manifest {
        /// Path to search from when locating brain.toml (walks up to find it).
        /// Defaults to the current directory.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Emit pretty-printed (indented) JSON instead of compact JSON.
        #[arg(long)]
        pretty: bool,
    },
    /// Generate derived views for the Bastion Brain repo (Phase 3, Block T).
    ///
    /// By default runs as a dry-run: prints the planned actions (W_EMIT_DRY_RUN) without
    /// writing any files. Pass --write to apply the changes in place (I_EMIT_WROTE per file).
    ///
    /// Derived views regenerated:
    ///   - Each leaf planning/state.json: `focus` (now/next/blocked) from tracks[].
    ///   - The brain planning/state.json: `repos[]` and `cross_repo[]` from children.
    ///   - Each sibling master-plan.md carrying <!-- BEGIN generated:wave-table --> sentinels:
    ///     the wave/dependency table is spliced in. Files without sentinels are skipped
    ///     with W_EMIT_NO_SENTINEL (never spliced into arbitrary prose).
    ///
    /// Diagnostic codes:
    ///   W_EMIT_DRY_RUN       — planned action (dry-run only; no file written)
    ///   I_EMIT_WROTE         — file written (--write mode)
    ///   W_EMIT_NO_SENTINEL   — master-plan.md missing sentinel pair; skipped
    ///   E_EMIT_WRITE_FAILED  — IO error writing a file (exit 1)
    ///   E_CONFIG_NOT_FOUND   — brain.toml could not be located (exit 1)
    ///   E_EMIT_LINKED_WORKTREE — --write invoked from inside a linked git worktree; refused
    ///                            before brain.toml resolution (exit 1). Dry-run is unaffected.
    ///   E_EMIT_INCOMPLETE_CORPUS — --write refused because a discovered state.json failed to
    ///                            load (exit 1); regenerating derived views from a partial
    ///                            corpus would silently erase the missing repo(s). Dry-run is
    ///                            unaffected — it still runs every planner and reports.
    ///   E_EMIT_UNKNOWN_SCOPE — --scope names a slug with no matching [[repos]] entry in
    ///                            brain.toml (exit 1); the diagnostic message names every
    ///                            valid slug.
    ///   E_EMIT_LOCK_HELD     — --write could not acquire the advisory lock at
    ///                            <root>/.mev-emit.lock because another live process
    ///                            already holds it (exit 1); a stale lock (owning process
    ///                            no longer alive) is reclaimed automatically instead.
    ///                            Dry-run never takes the lock.
    EmitState {
        /// Path to search from when locating brain.toml (walks up to find it).
        /// Defaults to the current directory.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Write the derived views in place. Without this flag the command is a dry-run:
        /// it prints what would be written (W_EMIT_DRY_RUN) without touching any files.
        #[arg(long)]
        write: bool,
        /// Limit regeneration to one repo's derived surfaces (its own leaf state.json,
        /// cache_doc, tier rollup, and the HQ board) plus nothing else — every other
        /// repo's files are left untouched. Omit to regenerate the whole corpus, which
        /// is today's default behaviour and stays byte-for-byte unchanged.
        #[arg(long, value_name = "REPO")]
        scope: Option<String>,
    },
    /// Emit the `scope:doc_id` knowledge graph as a JSON artifact (Phase 3B, Block R).
    ///
    /// Crawls the Brain repo, resolves `brain.toml`, builds the knowledge graph (nodes,
    /// `related:` edges, and leaves — no-`doc_id` corpus files), and prints a JSON envelope
    /// to stdout. Distinct from `generate-graph`, which writes an interactive HTML visual
    /// rather than a JSON artifact; this is the JSON companion for the orchestrator's
    /// Postgres edges table (D4). A pure emit — nothing is written to disk or a DB.
    ///
    /// Output is compact JSON by default; pass --pretty for indented output.
    ///
    /// Exit codes:
    ///   0 — graph emitted successfully
    ///   1 — configuration error (brain.toml not found or unreadable) or serialization error
    EmitGraph {
        /// Path to search from when locating brain.toml (walks up to find it).
        /// Defaults to the current directory.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Emit pretty-printed (indented) JSON instead of compact JSON.
        #[arg(long)]
        pretty: bool,
    },
    /// Emit the corpus-wide block-dependency graph as a JSON artifact (Phase 10, Block C).
    ///
    /// Crawls the Brain corpus, resolves `brain.toml`, loads every discovered
    /// `planning/state.json`, builds the block-dependency graph, and prints the enriched,
    /// scoped `BlockGraphExport` JSON envelope to stdout. This is the CLI companion to
    /// bastion's `GET /api/blocks/graph` (`BA.17.A`) — node counts for a given scope must
    /// match that endpoint's. A pure emit — nothing is written to disk.
    ///
    /// `--scope` selects the mode:
    ///   hq   (default) — every repo (`TierScope::All`)
    ///   tier            — repos in `--tier` (default `core`)
    ///   repo            — `--repo <SLUG>` intersected against the full corpus
    ///   epic            — `--epic <SLUG>` projection; overrides `--tier`/`--repo`
    ///
    /// Output is compact JSON by default; pass --pretty for indented output.
    ///
    /// Exit codes:
    ///   0 — graph emitted successfully
    ///   1 — configuration error (brain.toml not found or unreadable), `--scope epic` given
    ///       without `--epic`, `--scope repo` given without `--repo`, an unknown or blank
    ///       `--epic` slug, or a serialization error
    EmitBlockGraph {
        /// Path to search from when locating brain.toml (walks up to find it).
        /// Defaults to the current directory.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Scope mode: hq (default, every repo), tier, repo, or epic.
        #[arg(long, value_enum, default_value_t = BlockGraphScopeArg::Hq)]
        scope: BlockGraphScopeArg,
        /// Tier name to scope to; consulted only when --scope tier is given.
        #[arg(long, default_value = "core")]
        tier: String,
        /// Epic slug to project onto; required when --scope epic is given. Overrides tier.
        #[arg(long)]
        epic: Option<String>,
        /// Repo slug to intersect against; required when --scope repo is given.
        #[arg(long)]
        repo: Option<String>,
        /// Include closed blocks in the exported node set.
        #[arg(long)]
        include_closed: bool,
        /// Include out-of-scope boundary nodes (edges into/out of scope are retained).
        #[arg(long)]
        include_boundary: bool,
        /// Cap the exported node list at N nodes (topo-ordered); sets `truncated: true`
        /// when the pre-truncation node count exceeds N. Omit for no truncation.
        #[arg(long, value_name = "N")]
        max_nodes: Option<usize>,
        /// Emit pretty-printed (indented) JSON instead of compact JSON.
        #[arg(long)]
        pretty: bool,
    },
    /// Park an epic: set its registry status to `paused` and cascade `deferred`
    /// onto every one of its open member blocks.
    ///
    /// The epic-level counterpart of a block's `deferred` status. Parked work stays
    /// on the roadmap and stays counted, but stops competing for attention: its
    /// blocks leave `focus.next`, and its board section collapses to one line.
    ///
    /// Blocks with `status: "in_progress"` are deliberately left alone and reported
    /// (`W_EPIC_SKIPPED_IN_PROGRESS`) — parking work you are mid-block on is far more
    /// likely to be a mistake than an intent.
    ///
    /// Dry-run by default; pass --write to apply. A successful --write also runs
    /// `emit-state --write`, so `focus` and the boards are regenerated in the same
    /// invocation rather than being left drifted.
    ///
    /// `--write` takes the same advisory lock at <root>/.mev-emit.lock that
    /// `emit-state`/`set-block-status` take, before any file is touched. If another
    /// live process already holds it, this fails with E_EMIT_LOCK_HELD (naming the
    /// holder's pid) and writes nothing; a stale lock (owning process no longer
    /// alive) is reclaimed automatically instead of blocking. Dry-run never takes
    /// the lock and is unaffected by contention.
    ///
    /// Exit codes:
    ///   0 — planned (dry-run) or applied successfully
    ///   1 — unknown epic slug, no HQ registry, a write failure, or E_EMIT_LOCK_HELD
    DeferEpic {
        /// Epic slug as it appears in the HQ `epics[]` registry (e.g. `bastion-tui`).
        slug: String,
        /// Path to search from when locating brain.toml. Defaults to the current directory.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Apply the edits. Without this the command prints what it would change.
        #[arg(long)]
        write: bool,
    },
    /// Un-park an epic: set its registry status to `active` and return every
    /// `deferred` member block to `open`. The inverse of `defer-epic`.
    ///
    /// Dry-run by default; pass --write to apply (which also re-runs emit-state).
    ///
    /// `--write` takes the same advisory lock at <root>/.mev-emit.lock as
    /// `defer-epic`/`emit-state`/`set-block-status`; a held lock fails this with
    /// E_EMIT_LOCK_HELD and writes nothing, a stale lock is reclaimed automatically,
    /// and dry-run never takes the lock.
    ResumeEpic {
        /// Epic slug as it appears in the HQ `epics[]` registry.
        slug: String,
        /// Path to search from when locating brain.toml. Defaults to the current directory.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Apply the edits. Without this the command prints what it would change.
        #[arg(long)]
        write: bool,
    },
    /// Reconcile every epic's registry status against its blocks, in both directions.
    ///
    /// - An epic whose remaining work is entirely `deferred` but which is still
    ///   `active` is set to `paused`.
    /// - An epic already `paused` that still has `open` members has those members
    ///   deferred.
    ///
    /// Never un-defers anything: an active epic with *some* deferred blocks is a
    /// normal state, so un-parking stays explicit via `resume-epic`.
    ///
    /// Dry-run by default; pass --write to apply (which also re-runs emit-state).
    ///
    /// `--write` takes the same advisory lock at <root>/.mev-emit.lock as
    /// `defer-epic`/`resume-epic`/`emit-state`/`set-block-status`; a held lock fails
    /// this with E_EMIT_LOCK_HELD and writes nothing, a stale lock is reclaimed
    /// automatically, and dry-run never takes the lock.
    SyncEpics {
        /// Path to search from when locating brain.toml. Defaults to the current directory.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Apply the edits. Without this the command prints what it would change.
        #[arg(long)]
        write: bool,
    },
    /// Set one block's authored `status` in its repo's `planning/state.json`.
    ///
    /// The block-level counterpart to `defer-epic` / `resume-epic`: those move a whole
    /// initiative, this moves exactly one block and nothing else. Status only — not
    /// priority, not due, not a generic field setter.
    ///
    /// KEY is always `repo:id` (e.g. `mev:MV.10.A`). Block ids are only unique within
    /// a repo, so an unqualified id is rejected (`E_BLOCK_BAD_KEY`) rather than guessed.
    ///
    /// Valid STATUS values: `open`, `in_progress`, `deferred`, `closed`.
    /// `blocked` is deliberately NOT among them: it is a *derived* lane that emit-state
    /// computes from unmet dependencies, and authoring it onto a block is exactly what
    /// `E_STATE_AUTHORED_BLOCKED` rejects.
    ///
    /// Setting a block to the status it already has is a no-op success (exit 0, nothing
    /// written).
    ///
    /// Dry-run by default; pass --write to apply. A successful --write also runs
    /// `emit-state --write`, so `focus` and the boards are regenerated in the same
    /// invocation rather than being left drifted.
    ///
    /// The intended caller is an engine-rs workflow node acting for bastion-web —
    /// `bastion serve` stays read-only per D25, so block mutations are written here.
    ///
    /// Exit codes:
    ///   0 — planned (dry-run), applied, or already at the target status
    ///   1 — bad key, unauthorable status, unknown block, or a write failure
    SetBlockStatus {
        /// Block key in `repo:id` form, e.g. `mev:MV.10.A`.
        key: String,
        /// New authored status: open | in_progress | deferred | closed.
        status: String,
        /// Path to search from when locating brain.toml. Defaults to the current directory.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Apply the edit. Without this the command prints what it would change.
        #[arg(long)]
        write: bool,
    },
    /// Generate an interactive HTML visualization of the knowledge graph (graph.html)
    GenerateGraph {
        /// Path to search from when locating brain.toml. Defaults to the current directory.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// The output directory to write the graph files to (defaults to <root>/planning/doc-graph)
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Materialize brain documents and manage Opportunity records (Phase 9, Block MV.9.A).
    ///
    /// The generic doc-materializer over okf-core's `BrainDocModel`: `materialize` plans (and,
    /// with --write, applies) any of the three okf-core models from a raw JSON payload; the
    /// `opportunity` subcommand family (`ingest`, `set-stage`, `add-action`, `merge-contacts`)
    /// operates specifically on Opportunity documents under
    /// `business/docs/opportunities/`. Every verb resolves its target-corpus root via
    /// `find_brain_root` from the optional `path` argument (defaults to `.`).
    ///
    /// Dry-run is the default on every verb: without --write nothing is touched on disk and
    /// every planned action is still reported. Refuses --write from inside a linked git
    /// worktree, with the same guard message `emit-state` uses.
    ///
    /// Diagnostic codes:
    ///   W_DOC_UNCHANGED             — document already matches computed content (no-op)
    ///   W_DOC_MISSING_SENTINEL      — a generated section's sentinel pair is absent; that
    ///                                 section is left untouched rather than clobbered
    ///   W_DOC_INDEX_MISSING         — target index.md absent; no index action planned
    ///   W_DOC_INDEX_NO_TABLE        — index.md has no parsable table; no index action planned
    ///   W_DOC_INDEX_COLUMN_MISMATCH — row_cells count doesn't match the table's column count
    ///   E_DOC_BAD_INDEX_PATH        — model's index_path has no parent directory component
    ///   E_DOC_UNKNOWN_INPUT_SHAPE   — ingest input matches neither the company nor the
    ///                                 prospecting-sweep shape (pass --kind explicitly)
    ///   E_DOC_UNKNOWN_MODEL         — --model is not one of opportunity|learning-artifact|proposal
    ///   E_DOC_BAD_STAGE             — set-stage stage is not in the vocabulary authored in
    ///                                 business/docs/pipeline.md's `## Stages` line (D58)
    ///   E_DOC_NOT_FOUND             — a mutator's target file is absent or unparsable
    ///   W_EMIT_DRY_RUN / I_EMIT_WROTE — reused unchanged from `apply_plan`'s write half
    ///
    /// Exit codes:
    ///   0 — planned (dry-run) or applied successfully, no errors
    ///   1 — a resolution/parse/write failure, a linked-worktree write refusal, or any
    ///       error-severity diagnostic (`E_DOC_*` / `E_CONFIG_NOT_FOUND`)
    Doc {
        #[command(subcommand)]
        command: DocCommand,
    },
}

#[derive(Subcommand)]
enum DocCommand {
    /// Plan (and, with --write, apply) a document for one of the three okf-core models from a
    /// raw JSON payload.
    Materialize {
        /// Which okf-core `BrainDocModel` to build: opportunity | learning-artifact | proposal.
        #[arg(long)]
        model: String,
        /// Path to the JSON payload to build the model from.
        #[arg(long)]
        input: PathBuf,
        /// Path to search from when locating brain.toml. Defaults to the current directory.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Apply the write. Without this the command is a dry-run.
        #[arg(long)]
        write: bool,
    },
    /// The Opportunity command family: ingest / set-stage / add-action / merge-contacts.
    Opportunity {
        #[command(subcommand)]
        command: OpportunityCommand,
    },
}

#[derive(Subcommand)]
enum OpportunityCommand {
    /// Ingest a raw CompanyBrief/ProspectingResult/job-posting JSON payload as a new (or
    /// updated) Opportunity document.
    Ingest {
        /// Path to the JSON payload (CompanyBrief or ProspectingResult shape).
        #[arg(long)]
        input: PathBuf,
        /// Explicit opportunity kind: company | prospecting-sweep | job-posting. When omitted,
        /// the kind is auto-detected from the input's shape.
        #[arg(long)]
        kind: Option<String>,
        /// Path to search from when locating brain.toml. Defaults to the current directory.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Apply the write. Without this the command is a dry-run.
        #[arg(long)]
        write: bool,
    },
    /// Set an existing opportunity's `stage`.
    SetStage {
        /// The opportunity's slug (its filename stem under business/docs/opportunities/).
        slug: String,
        /// The new stage: identified | researching | contacted | conversation |
        /// proposal-sent | closed-won | closed-lost.
        stage: String,
        /// Path to search from when locating brain.toml. Defaults to the current directory.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Apply the write. Without this the command is a dry-run.
        #[arg(long)]
        write: bool,
    },
    /// Append one `{at, kind, note}` action to an existing opportunity's `actions[]`.
    AddAction {
        /// The opportunity's slug.
        slug: String,
        /// The action's kind (e.g. "email", "call", "meeting").
        #[arg(long)]
        kind: String,
        /// A free-form note describing the action.
        #[arg(long)]
        note: String,
        /// The action's ISO date. Defaults to today when omitted.
        #[arg(long)]
        at: Option<String>,
        /// Path to search from when locating brain.toml. Defaults to the current directory.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Apply the write. Without this the command is a dry-run.
        #[arg(long)]
        write: bool,
    },
    /// Merge one or more contacts into an existing opportunity's `contacts[]`, matched on
    /// `name`.
    MergeContacts {
        /// The opportunity's slug.
        slug: String,
        /// Path to a JSON contact object, or a JSON array of contact objects.
        #[arg(long)]
        input: PathBuf,
        /// Path to search from when locating brain.toml. Defaults to the current directory.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Apply the write. Without this the command is a dry-run.
        #[arg(long)]
        write: bool,
    },
}

/// Shared dispatch for `defer-epic` / `resume-epic` / `sync-epics`.
///
/// All three resolve the brain root, run [`mev::epic_status`], and report in the
/// same shape as `emit-state` (per-diagnostic lines + a mode/count summary, or a
/// JSON envelope under `--json`).
fn run_epic_status(
    path: &std::path::Path,
    slug: Option<&str>,
    action: mev::brain::epics::EpicAction,
    write: bool,
    json: bool,
) -> ExitCode {
    // Same worktree guard as emit-state: a --write here chains into emit-state,
    // which resolves every repo's paths from brain.toml rather than CWD.
    if write && mev::brain::config::is_linked_worktree(path) {
        eprintln!(
            "error: refusing to write from inside a linked git worktree ({}) — epic edits chain \
             into emit-state, which resolves derived-file paths from brain.toml, not CWD, so this \
             would regenerate the MAIN checkout's files. Run from the main working tree instead.",
            path.display()
        );
        return ExitCode::FAILURE;
    }

    let root = match mev::brain::config::find_brain_root(path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    let label = match (slug, action) {
        (Some(_), mev::brain::epics::EpicAction::Defer) => "defer-epic",
        (Some(_), mev::brain::epics::EpicAction::Resume) => "resume-epic",
        (None, _) => "sync-epics",
    };

    // Advisory lock, same contract as emit-state/set-block-status: only --write mutates
    // the corpus, so only --write needs mutual exclusion. This command writes an
    // *authored* status field and then chains into emit-state, so racing it against a
    // concurrent emit would let the derived views be regenerated mid-edit. Released via
    // Drop on every exit path below.
    let _lock_guard = if write {
        match mev::brain::lock::acquire_lock(&root, mev::brain::lock::DEFAULT_LOCK_TIMEOUT) {
            Ok(guard) => Some(guard),
            Err(mev::brain::lock::LockError::Held {
                holder_pid,
                lock_path,
                waited_secs,
            }) => {
                eprintln!(
                    "error [E_EMIT_LOCK_HELD] another write (pid {holder_pid}) holds the lock at {} after waiting {waited_secs}s; retry once it finishes.",
                    lock_path.display()
                );
                return ExitCode::FAILURE;
            }
            Err(e) => {
                eprintln!("error [E_EMIT_LOCK_HELD] {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        None
    };

    match mev::epic_status(&root, slug, action, write) {
        Ok(report) => {
            if json {
                let envelope = mev::JsonReport::new(label, &root, &report);
                match envelope.to_json() {
                    Ok(s) => println!("{s}"),
                    Err(e) => {
                        eprintln!("error: could not serialize report: {e}");
                        return ExitCode::FAILURE;
                    }
                }
            } else {
                for d in &report.diagnostics {
                    print_diagnostic(d);
                }
                let mode = if write { "write" } else { "dry-run" };
                println!(
                    "{label} {} {}: {} error(s), {} warning(s)",
                    mode,
                    root.display(),
                    report.error_count(),
                    report.warning_count()
                );
            }
            if report.is_failure() {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

/// Shared guard for every `mev doc ...` verb: refuse `--write` from inside a linked git
/// worktree, same message shape as `emit-state`/`run_epic_status`. Returns `Some(exit code)`
/// when the guard fires (caller should return it immediately), `None` otherwise.
fn doc_worktree_guard(path: &std::path::Path, write: bool) -> Option<ExitCode> {
    if write && mev::brain::config::is_linked_worktree(path) {
        eprintln!(
            "error: refusing to write from inside a linked git worktree ({}) — mev doc resolves \
             derived-file paths from brain.toml, not CWD, so writing from a worktree would \
             silently regenerate the MAIN checkout's files instead of the worktree's own copy. \
             Run `mev doc ... --write` from the main working tree instead.",
            path.display()
        );
        return Some(ExitCode::FAILURE);
    }
    None
}

/// Resolve the brain root from `path`, printing `error: {e}` and returning the failure exit
/// code on failure.
fn doc_resolve_root(path: &std::path::Path) -> Result<PathBuf, ExitCode> {
    mev::brain::config::find_brain_root(path).map_err(|e| {
        eprintln!("error: {e}");
        ExitCode::FAILURE
    })
}

/// Read and parse a JSON payload from `path`, printing `error: {e}` and returning the failure
/// exit code on failure.
fn doc_read_json(path: &std::path::Path) -> Result<serde_json::Value, ExitCode> {
    let content = std::fs::read_to_string(path).map_err(|e| {
        eprintln!("error: could not read {}: {e}", path.display());
        ExitCode::FAILURE
    })?;
    serde_json::from_str(&content).map_err(|e| {
        eprintln!("error: could not parse {} as JSON: {e}", path.display());
        ExitCode::FAILURE
    })
}

/// Shared reporting tail for every `mev doc ...` verb: print diagnostics (or a `--json`
/// envelope), then a `<label> <mode> <root>: N error(s), M warning(s)` summary, and translate
/// the report's failure state into the process exit code.
fn report_doc(
    label: &str,
    root: &std::path::Path,
    write: bool,
    json: bool,
    result: anyhow::Result<mev::Report>,
) -> ExitCode {
    match result {
        Ok(report) => {
            if json {
                let envelope = mev::JsonReport::new(label, root, &report);
                match envelope.to_json() {
                    Ok(s) => println!("{s}"),
                    Err(e) => {
                        eprintln!("error: could not serialize report: {e}");
                        return ExitCode::FAILURE;
                    }
                }
            } else {
                for d in &report.diagnostics {
                    print_diagnostic(d);
                }
                let mode = if write { "write" } else { "dry-run" };
                println!(
                    "{label} {} {}: {} error(s), {} warning(s)",
                    mode,
                    root.display(),
                    report.error_count(),
                    report.warning_count()
                );
            }
            if report.is_failure() {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn print_diagnostic(d: &mev::Diagnostic) {
    let sev = match d.severity {
        Severity::Error => theme::severity_error("error"),
        Severity::Warning => theme::severity_warning("warning"),
    };
    println!(
        "{} [{}] {} — {}",
        sev,
        theme::locator(&d.locator),
        d.file.display(),
        theme::message(&d.message),
    );
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Validate { path } => match mev::validate(&path) {
            Ok(report) => {
                if cli.json {
                    let envelope = mev::JsonReport::new("learn-ai", &path, &report);
                    match envelope.to_json() {
                        Ok(s) => println!("{s}"),
                        Err(err) => {
                            eprintln!("error serializing JSON: {err:#}");
                            return ExitCode::FAILURE;
                        }
                    }
                } else {
                    for d in &report.diagnostics {
                        print_diagnostic(d);
                    }
                    println!(
                        "validated {}: {} error(s), {} warning(s)",
                        path.display(),
                        report.error_count(),
                        report.warning_count()
                    );
                }
                if report.is_failure() {
                    ExitCode::FAILURE
                } else {
                    ExitCode::SUCCESS
                }
            }
            Err(err) => {
                eprintln!("error: {err:#}");
                ExitCode::FAILURE
            }
        },
        Command::ValidateBrain {
            path,
            sync,
            graph,
            state,
            links,
            structure,
        } => {
            let root = match mev::brain::config::find_brain_root(&path) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::FAILURE;
                }
            };
            let result = if links {
                mev::validate_brain_links(&root)
            } else if structure {
                mev::validate_brain_structure(&root)
            } else if state {
                mev::validate_brain_state(&root)
            } else if graph {
                mev::validate_brain_graph(&root)
            } else if sync {
                mev::validate_brain_sync(&root)
            } else {
                mev::validate_brain(&root)
            };
            match result {
                Ok(report) => {
                    if cli.json {
                        let envelope = mev::JsonReport::new("brain", &root, &report);
                        match envelope.to_json() {
                            Ok(s) => println!("{s}"),
                            Err(err) => {
                                eprintln!("error serializing JSON: {err:#}");
                                return ExitCode::FAILURE;
                            }
                        }
                    } else {
                        for d in &report.diagnostics {
                            print_diagnostic(d);
                        }
                        println!(
                            "validated {}: {} error(s), {} warning(s)",
                            root.display(),
                            report.error_count(),
                            report.warning_count()
                        );
                    }
                    if report.is_failure() {
                        ExitCode::FAILURE
                    } else {
                        ExitCode::SUCCESS
                    }
                }
                Err(err) => {
                    eprintln!("error: {err:#}");
                    ExitCode::FAILURE
                }
            }
        }
        Command::EmitState { path, write, scope } => {
            if write && mev::brain::config::is_linked_worktree(&path) {
                eprintln!(
                    "error: refusing to run emit-state --write from inside a linked git worktree ({}) — emit-state resolves every repo's derived-file paths from brain.toml, not CWD, so writing from a worktree would silently regenerate the MAIN checkout's files instead of the worktree's own copy. Run `mev emit-state --write` from the main working tree instead.",
                    path.display()
                );
                return ExitCode::FAILURE;
            }
            let root = match mev::brain::config::find_brain_root(&path) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::FAILURE;
                }
            };
            // Advisory lock: only --write mutates derived files, so only --write needs
            // mutual exclusion. Dry-run stays lock-free (it never touches disk). The
            // guard is held for the rest of this match arm and releases on every exit
            // path (success or error) via Drop.
            let _lock_guard = if write {
                match mev::brain::lock::acquire_lock(&root, mev::brain::lock::DEFAULT_LOCK_TIMEOUT)
                {
                    Ok(guard) => Some(guard),
                    Err(mev::brain::lock::LockError::Held {
                        holder_pid,
                        lock_path,
                        waited_secs,
                    }) => {
                        eprintln!(
                            "error [E_EMIT_LOCK_HELD] another emit-state --write (pid {holder_pid}) holds the lock at {} after waiting {waited_secs}s; retry once it finishes.",
                            lock_path.display()
                        );
                        return ExitCode::FAILURE;
                    }
                    Err(e) => {
                        eprintln!("error [E_EMIT_LOCK_HELD] {e}");
                        return ExitCode::FAILURE;
                    }
                }
            } else {
                None
            };
            let scope_deps = match &scope {
                Some(slug) => {
                    let config =
                        match mev::brain::config::load_brain_config(&root.join("brain.toml")) {
                            Ok(cfg) => cfg,
                            Err(e) => {
                                eprintln!("error: {e}");
                                return ExitCode::FAILURE;
                            }
                        };
                    match config.scope_dependencies(slug) {
                        Ok(deps) => Some(deps),
                        Err(mev::brain::config::ScopeError::UnknownSlug { slug, valid_slugs }) => {
                            eprintln!(
                                "error [E_EMIT_UNKNOWN_SCOPE] unknown --scope slug '{slug}'; valid slugs: {}",
                                valid_slugs.join(", ")
                            );
                            return ExitCode::FAILURE;
                        }
                        Err(e) => {
                            eprintln!("error [E_EMIT_UNKNOWN_SCOPE] {e}");
                            return ExitCode::FAILURE;
                        }
                    }
                }
                None => None,
            };
            match mev::emit_state(&root, write, scope_deps.as_ref()) {
                Ok(report) => {
                    if cli.json {
                        let envelope = mev::JsonReport::new("brain-emit", &root, &report);
                        match envelope.to_json() {
                            Ok(s) => println!("{s}"),
                            Err(err) => {
                                eprintln!("error serializing JSON: {err:#}");
                                return ExitCode::FAILURE;
                            }
                        }
                    } else {
                        for d in &report.diagnostics {
                            print_diagnostic(d);
                        }
                        let mode = if write { "write" } else { "dry-run" };
                        println!(
                            "emit-state {} {}: {} error(s), {} warning(s)",
                            mode,
                            root.display(),
                            report.error_count(),
                            report.warning_count()
                        );
                    }
                    if report.is_failure() {
                        ExitCode::FAILURE
                    } else {
                        ExitCode::SUCCESS
                    }
                }
                Err(err) => {
                    eprintln!("error: {err:#}");
                    ExitCode::FAILURE
                }
            }
        }
        Command::DeferEpic { slug, path, write } => run_epic_status(
            &path,
            Some(&slug),
            mev::brain::epics::EpicAction::Defer,
            write,
            cli.json,
        ),
        Command::ResumeEpic { slug, path, write } => run_epic_status(
            &path,
            Some(&slug),
            mev::brain::epics::EpicAction::Resume,
            write,
            cli.json,
        ),
        Command::SyncEpics { path, write } => run_epic_status(
            &path,
            None,
            mev::brain::epics::EpicAction::Defer,
            write,
            cli.json,
        ),
        Command::SetBlockStatus {
            key,
            status,
            path,
            write,
        } => {
            // Same worktree guard as emit-state: a --write here chains into emit-state,
            // which resolves every repo's paths from brain.toml rather than CWD.
            if write && mev::brain::config::is_linked_worktree(&path) {
                eprintln!(
                    "error: refusing to write from inside a linked git worktree ({}) — set-block-status chains into emit-state, which resolves derived-file paths from brain.toml, not CWD, so this would regenerate the MAIN checkout's files. Run from the main working tree instead.",
                    path.display()
                );
                return ExitCode::FAILURE;
            }
            let root = match mev::brain::config::find_brain_root(&path) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::FAILURE;
                }
            };
            // Advisory lock, same contract as emit-state: only --write mutates the
            // corpus, so only --write needs mutual exclusion. This command writes an
            // *authored* field and then chains into emit-state, so racing it against a
            // concurrent emit would let the derived views be regenerated mid-edit.
            // Released via Drop on every exit path below.
            let _lock_guard = if write {
                match mev::brain::lock::acquire_lock(&root, mev::brain::lock::DEFAULT_LOCK_TIMEOUT)
                {
                    Ok(guard) => Some(guard),
                    Err(mev::brain::lock::LockError::Held {
                        holder_pid,
                        lock_path,
                        waited_secs,
                    }) => {
                        eprintln!(
                            "error [E_EMIT_LOCK_HELD] another write (pid {holder_pid}) holds the lock at {} after waiting {waited_secs}s; retry once it finishes.",
                            lock_path.display()
                        );
                        return ExitCode::FAILURE;
                    }
                    Err(e) => {
                        eprintln!("error [E_EMIT_LOCK_HELD] {e}");
                        return ExitCode::FAILURE;
                    }
                }
            } else {
                None
            };
            report_doc(
                "set-block-status",
                &root,
                write,
                cli.json,
                mev::set_block_status(&root, &key, &status, write),
            )
        }
        Command::Manifest { path, pretty } => {
            let root = match mev::brain::config::find_brain_root(&path) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::FAILURE;
                }
            };
            match mev::manifest_brain(&root) {
                Ok(manifest) => {
                    let json_result = if pretty {
                        serde_json::to_string_pretty(&manifest)
                    } else {
                        serde_json::to_string(&manifest)
                    };
                    match json_result {
                        Ok(s) => {
                            println!("{s}");
                            ExitCode::SUCCESS
                        }
                        Err(err) => {
                            eprintln!("error serializing manifest: {err:#}");
                            ExitCode::FAILURE
                        }
                    }
                }
                Err(err) => {
                    eprintln!("error: {err:#}");
                    ExitCode::FAILURE
                }
            }
        }
        Command::EmitGraph { path, pretty } => {
            let root = match mev::brain::config::find_brain_root(&path) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::FAILURE;
                }
            };
            match mev::graph_brain(&root) {
                Ok(export) => {
                    let json_result = if pretty {
                        serde_json::to_string_pretty(&export)
                    } else {
                        serde_json::to_string(&export)
                    };
                    match json_result {
                        Ok(s) => {
                            println!("{s}");
                            ExitCode::SUCCESS
                        }
                        Err(err) => {
                            eprintln!("error serializing graph: {err:#}");
                            ExitCode::FAILURE
                        }
                    }
                }
                Err(err) => {
                    eprintln!("error: {err:#}");
                    ExitCode::FAILURE
                }
            }
        }
        Command::EmitBlockGraph {
            path,
            scope,
            tier,
            epic,
            repo,
            include_closed,
            include_boundary,
            max_nodes,
            pretty,
        } => {
            if scope == BlockGraphScopeArg::Epic && epic.is_none() {
                eprintln!("error: --scope epic requires --epic <SLUG>");
                return ExitCode::FAILURE;
            }
            if scope == BlockGraphScopeArg::Repo && repo.is_none() {
                eprintln!("error: --scope repo requires --repo <SLUG>");
                return ExitCode::FAILURE;
            }

            let root = match mev::brain::config::find_brain_root(&path) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::FAILURE;
                }
            };

            let tier_scope = match scope {
                BlockGraphScopeArg::Tier => mev::brain::state::TierScope::Tier(tier),
                BlockGraphScopeArg::Hq | BlockGraphScopeArg::Repo | BlockGraphScopeArg::Epic => {
                    mev::brain::state::TierScope::All
                }
            };
            let repo_filter = if scope == BlockGraphScopeArg::Repo {
                repo
            } else {
                None
            };
            let epic_filter = if scope == BlockGraphScopeArg::Epic {
                epic
            } else {
                None
            };

            let block_scope = mev::brain::block_graph::BlockGraphScope {
                tier: tier_scope,
                epic: epic_filter,
                repo: repo_filter,
                include_closed,
                include_boundary,
                max_nodes: max_nodes.unwrap_or(usize::MAX),
            };

            match mev::block_graph_brain(&root, &block_scope) {
                Ok(export) => {
                    let json_result = if pretty {
                        serde_json::to_string_pretty(&export)
                    } else {
                        serde_json::to_string(&export)
                    };
                    match json_result {
                        Ok(s) => {
                            println!("{s}");
                            ExitCode::SUCCESS
                        }
                        Err(err) => {
                            eprintln!("error serializing block graph: {err:#}");
                            ExitCode::FAILURE
                        }
                    }
                }
                Err(err) => {
                    eprintln!("error: {err:#}");
                    ExitCode::FAILURE
                }
            }
        }
        Command::Doc { command } => match command {
            DocCommand::Materialize {
                model,
                input,
                path,
                write,
            } => {
                if let Some(code) = doc_worktree_guard(&path, write) {
                    return code;
                }
                let root = match doc_resolve_root(&path) {
                    Ok(r) => r,
                    Err(code) => return code,
                };
                let input_json = match doc_read_json(&input) {
                    Ok(v) => v,
                    Err(code) => return code,
                };
                let result = mev::doc_materialize(&root, &model, &input_json, write);
                report_doc("doc-materialize", &root, write, cli.json, result)
            }
            DocCommand::Opportunity { command } => match command {
                OpportunityCommand::Ingest {
                    input,
                    kind,
                    path,
                    write,
                } => {
                    if let Some(code) = doc_worktree_guard(&path, write) {
                        return code;
                    }
                    let root = match doc_resolve_root(&path) {
                        Ok(r) => r,
                        Err(code) => return code,
                    };
                    let input_json = match doc_read_json(&input) {
                        Ok(v) => v,
                        Err(code) => return code,
                    };
                    let kind = match kind {
                        Some(k) => match k.parse::<mev::OpportunityKind>() {
                            Ok(k) => Some(k),
                            Err(e) => {
                                eprintln!("error: {e}");
                                return ExitCode::FAILURE;
                            }
                        },
                        None => None,
                    };
                    let result = mev::doc_opportunity_ingest(&root, &input_json, kind, write);
                    report_doc("doc-opportunity-ingest", &root, write, cli.json, result)
                }
                OpportunityCommand::SetStage {
                    slug,
                    stage,
                    path,
                    write,
                } => {
                    if let Some(code) = doc_worktree_guard(&path, write) {
                        return code;
                    }
                    let root = match doc_resolve_root(&path) {
                        Ok(r) => r,
                        Err(code) => return code,
                    };
                    let result = mev::doc_opportunity_set_stage(&root, &slug, &stage, write);
                    report_doc("doc-opportunity-set-stage", &root, write, cli.json, result)
                }
                OpportunityCommand::AddAction {
                    slug,
                    kind,
                    note,
                    at,
                    path,
                    write,
                } => {
                    if let Some(code) = doc_worktree_guard(&path, write) {
                        return code;
                    }
                    let root = match doc_resolve_root(&path) {
                        Ok(r) => r,
                        Err(code) => return code,
                    };
                    let at = at.unwrap_or_else(|| chrono::Local::now().date_naive().to_string());
                    let result =
                        mev::doc_opportunity_add_action(&root, &slug, &at, &kind, &note, write);
                    report_doc("doc-opportunity-add-action", &root, write, cli.json, result)
                }
                OpportunityCommand::MergeContacts {
                    slug,
                    input,
                    path,
                    write,
                } => {
                    if let Some(code) = doc_worktree_guard(&path, write) {
                        return code;
                    }
                    let root = match doc_resolve_root(&path) {
                        Ok(r) => r,
                        Err(code) => return code,
                    };
                    let input_json = match doc_read_json(&input) {
                        Ok(v) => v,
                        Err(code) => return code,
                    };
                    let result =
                        mev::doc_opportunity_merge_contacts(&root, &slug, &input_json, write);
                    report_doc(
                        "doc-opportunity-merge-contacts",
                        &root,
                        write,
                        cli.json,
                        result,
                    )
                }
            },
        },
        Command::GenerateGraph { path, out } => {
            let root = match mev::brain::config::find_brain_root(&path) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::FAILURE;
                }
            };
            match mev::visualize_brain(&root, out) {
                Ok(_) => ExitCode::SUCCESS,
                Err(err) => {
                    eprintln!("error: {err:#}");
                    ExitCode::FAILURE
                }
            }
        }
    }
}
