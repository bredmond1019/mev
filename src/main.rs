//! `mev` CLI entry point. Thin wrapper over the library: parse args, dispatch, set exit code.

use std::io::IsTerminal;
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

    /// Print this binary's build provenance (git_sha, dirty, source_dir) as one JSON line
    /// to stdout and exit immediately — before any subcommand runs. This is the
    /// cross-binary contract `toolchain-freshness` uses to query other registered corpus
    /// writers (see `MV.ticket.toolchain-freshness-covers-the-writer`); do not add, rename,
    /// or drop a key from the emitted shape.
    #[arg(long, global = true)]
    build_stamp: bool,

    /// Optional so `mev --build-stamp` can run with no subcommand at all; every other
    /// invocation still requires one (enforced in `main()`, since clap can't express
    /// "required unless --build-stamp" declaratively here).
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Validate the learn-ai content tree (Phase 1: learn modules).
    ///
    /// With --blog, validates the learn-ai blog tree instead (Phase 12, Block A): EN posts under
    /// `blog/published/*.mdx` and pt-BR posts under `blog/published/pt-BR/*.mdx`. Surfaces
    /// `E_BLOG_MALFORMED_FRONTMATTER`, `E_BLOG_MISSING_FIELD`, `W_BLOG_PTBR_MISSING`, and the
    /// shared lint codes (`W_LINT_UNTAGGED_CODE_BLOCK`, `E_LINT_DEAD_LOCAL_LINK`,
    /// `E_LINT_DEAD_ASSET`), which run on by default for blog posts. This is a **content**
    /// check — it surfaces here, never through `validate-brain`.
    ///
    /// With --lint (and without --blog), additionally runs the shared content-lint passes over
    /// learn modules, reporting `W_LINT_UNTAGGED_CODE_BLOCK` / `E_LINT_DEAD_LOCAL_LINK` /
    /// `E_LINT_DEAD_ASSET`. A no-op when combined with --blog, since lint is already on there.
    /// Without either flag, behaviour is byte-identical to the pre-Phase-12 binary.
    Validate {
        /// Path to the content root. Defaults to ../learn-ai/content/learn, or, when --blog is
        /// given, to ../learn-ai/content/blog/published.
        path: Option<PathBuf>,
        /// Validate the learn-ai blog tree instead of the learn module tree. Changes the
        /// positional path's default and the --json consumer label to "blog". Runs the shared
        /// lint passes (untagged code blocks, dead local links/assets) on by default alongside
        /// the blog-specific frontmatter and pt-BR parity checks.
        #[arg(long)]
        blog: bool,
        /// Run the shared content-lint passes (W_LINT_UNTAGGED_CODE_BLOCK,
        /// E_LINT_DEAD_LOCAL_LINK, E_LINT_DEAD_ASSET) over learn modules. Ignored (no-op) when
        /// --blog is also given, since lint already runs there by default. Without this flag,
        /// `mev validate` stays byte-identical to the pre-Phase-12 binary.
        #[arg(long)]
        lint: bool,
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
    /// Validate a single `state.json` file (Phase 3, Block E:
    /// `ticket-reference-container-validation` Task 5).
    ///
    /// The single-file sibling of `mev validate-brain --state`: runs only the
    /// per-file schema/field-policy ring (load, `check_schema`, `check_field_policy`)
    /// against exactly one file, and deliberately skips every corpus-level check
    /// (block graph, cycles, rollup drift, focus drift, status consistency) — those
    /// need sibling repos to evaluate and cannot run from one file in isolation.
    /// Cheap enough to run after every manual `state.json` edit, which is what would
    /// have caught the live 2026-08-13 shape-error incident (`scope` authored as a
    /// plain string, `related` as bare slug strings) before it cascaded to 50 errors
    /// across 7 files.
    ///
    /// Exit codes:
    ///   0 — the file loaded cleanly and no error-severity diagnostic was raised
    ///       (warnings alone do not fail the run)
    ///   1 — the file is missing, is not valid JSON, fails the typed schema, or an
    ///       error-severity diagnostic was raised
    ValidateState {
        /// Path to the `state.json` file to validate.
        path: PathBuf,
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
        /// Promote a `toolchain-freshness` Drift verdict from a warning to a hard
        /// failure: no write performed, non-zero exit. Convenience alias for setting the
        /// `MEV_REQUIRE_FRESH` env var before this write runs — see `mev::emit_state`'s
        /// doc comment. `NotEvaluable` never triggers this; only a genuine Drift does.
        #[arg(long)]
        require_fresh: bool,
    },
    /// List (or restore) the append-only revision history `apply_plan()` records for
    /// one file every time it overwrites existing content (see `emit-state`'s
    /// "Revision history" note above, and `mev::brain::history`).
    ///
    /// Default (no `--restore`): prints that file's revisions NEWEST FIRST — seq, UTC
    /// timestamp, byte size. A file with no recorded revisions prints an explicit
    /// "no revisions recorded" message and exits successfully (an empty history is a
    /// normal state, not an error). Read-only: never takes the advisory lock.
    ///
    /// `--restore <SEQ>`: first snapshots the file's current on-disk content as a new
    /// revision (so a wrong restore is itself undoable via a second restore), then
    /// writes revision `<SEQ>`'s content back to `<path>` atomically via the same
    /// temp-file + rename helper `apply_plan()` uses. Mutates the file, so it takes
    /// the same advisory lock at `<root>/.mev-emit.lock` that `emit-state --write`
    /// takes, and the same linked-worktree guard.
    ///
    /// Diagnostic codes:
    ///   E_EMIT_LINKED_WORKTREE — --restore invoked from inside a linked git worktree;
    ///                            refused before the lock is taken.
    ///   E_EMIT_LOCK_HELD       — --restore could not acquire the advisory lock because
    ///                            another live write process already holds it (exit 1);
    ///                            a stale lock (owning process no longer alive) is
    ///                            reclaimed automatically instead.
    ///   W_HISTORY_FAILED       — the pre-restore snapshot could not be recorded; the
    ///                            restore itself still proceeds (history is a safety
    ///                            net, never a new way for restore to fail).
    ///
    /// Exit codes:
    ///   0 — revisions listed, "no revisions recorded", or restore applied
    ///   1 — no revision `<SEQ>` on disk (names the valid seq range), E_EMIT_LOCK_HELD,
    ///       a linked-worktree refusal, or an IO failure reading/writing the file
    StateHistory {
        /// The file whose revision history to list or restore (e.g. planning/state.json).
        path: PathBuf,
        /// Restore revision SEQ's content back to `path` instead of listing.
        #[arg(long, value_name = "SEQ")]
        restore: Option<u32>,
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
    /// Declare an initiative finished: set its registry status to `complete`.
    ///
    /// This is an **operator declaration**, not an inference. `mev` never auto-flips
    /// an epic to `complete` on your behalf — `W_STATE_EPIC_ALL_CLOSED` is warn-only
    /// by design (state-schema.md:290) precisely because the last member block
    /// closing is not the same as the initiative's goal being met. This command is
    /// the sanctioned way to state that judgement explicitly, by name, on the
    /// command line; it never inspects member status and is compatible with, not a
    /// workaround for, that rule.
    ///
    /// Sets **only** the named epic's registry status — no member block's status is
    /// ever touched. `complete` is terminal and drops the epic off the board; its
    /// members' own statuses remain whatever they already were.
    ///
    /// Dry-run by default; pass --write to apply (which also re-runs emit-state).
    ///
    /// `--write` takes the same advisory lock at <root>/.mev-emit.lock as
    /// `defer-epic`/`resume-epic`/`sync-epics`/`emit-state`; a held lock fails this
    /// with E_EMIT_LOCK_HELD and writes nothing, a stale lock is reclaimed
    /// automatically, and dry-run never takes the lock. Refused the same way as its
    /// siblings when run from inside a linked git worktree.
    ///
    /// Exit codes:
    ///   0 — planned (dry-run) or applied successfully, including a no-op on an
    ///       epic already `complete`
    ///   1 — unknown epic slug, no HQ registry, a write failure, or E_EMIT_LOCK_HELD
    CompleteEpic {
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
    /// **Operator gate (D71).** A `--write` that would start a block (set it to
    /// `in_progress`) while it still carries an unmet `operator` depends_on entry
    /// is refused with `E_BLOCK_OPERATOR_GATED`. The only override is
    /// `--force-operator-gate`, and that flag is itself human-only: it is refused
    /// with `E_FORCE_OPERATOR_GATE_NOT_TTY` whenever stdin is not a TTY, so an
    /// agent can never pass it to clear its own gate. There is no priority
    /// threshold or other bypass.
    ///
    /// Exit codes:
    ///   0 — planned (dry-run), applied, or already at the target status
    ///   1 — bad key, unauthorable status, unknown block, a write failure,
    ///       an unmet operator gate without --force-operator-gate,
    ///       --force-operator-gate on non-TTY stdin, or E_EMIT_UNKNOWN_SCOPE
    ///
    /// **Fleet coupling (unchanged by `--scope`).** `E_EMIT_INCOMPLETE_CORPUS` still
    /// aborts the whole write whenever ANY state.json fleet-wide fails to load — the
    /// completeness guard runs before the plan is built, regardless of `--scope`.
    /// `--scope` only narrows which derived surfaces the *chained emit* regenerates
    /// once the authored write has already succeeded; it does not narrow which
    /// repos' state.json files must load cleanly for that write to be allowed at all.
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
        /// Human-only override (D71) to start a block despite an unmet `operator`
        /// depends_on edge. Refused with `E_FORCE_OPERATOR_GATE_NOT_TTY` when
        /// stdin is not a TTY — an agent can never pass this flag to clear its
        /// own gate.
        #[arg(long = "force-operator-gate")]
        force_operator_gate: bool,
        /// Limit the chained emit's regeneration to one repo's derived surfaces (its
        /// own leaf state.json, cache_doc, tier rollup, and the HQ board) plus
        /// nothing else — every other repo's files are left untouched. Omit to
        /// regenerate the whole corpus, which is today's default behaviour and
        /// stays byte-for-byte unchanged. Resolved the same way as `emit-state
        /// --scope`; an unknown or blank slug exits with `E_EMIT_UNKNOWN_SCOPE`.
        #[arg(long, value_name = "REPO")]
        scope: Option<String>,
    },
    /// Close an operator gate fleet-wide: remove every `depends_on` `{type:"operator"}`
    /// entry carrying SLUG, across every loaded `state.json`.
    ///
    /// One `slug` can gate several blocks (even across repos) — this clears all of
    /// them in a single call, never one block at a time.
    ///
    /// **Not** dry-run/`--write` shaped like `defer-epic`/`set-block-status`: this
    /// command is verified-or-refused. It **refuses unless `--exit-verified` is
    /// passed** — the operator edge's `exit` field names an artifact whose existence
    /// ends the gate, and mev never checks the filesystem for it. `--exit-verified`
    /// is the caller's plain assertion that they looked; refusing without it (rather
    /// than defaulting to a dry-run) is what keeps this a human gate. Nothing is
    /// read or touched when the flag is absent.
    ///
    /// SLUG matching no operator edge in the loaded corpus is an error, not a silent
    /// no-op — almost always a typo.
    ///
    /// On success, re-runs `emit-state --write` so `focus`/the boards agree with the
    /// cleared gate, under the same `<root>/.mev-emit.lock` advisory lock every other
    /// authored-state writer takes (E_EMIT_LOCK_HELD on contention; a stale lock from
    /// a dead pid is reclaimed automatically). Refused the same way as its siblings
    /// when run from inside a linked git worktree.
    ///
    /// Exit codes:
    ///   0 — every matching edge removed and emit-state re-run cleanly
    ///   1 — missing --exit-verified, unknown slug, a write failure, E_EMIT_LOCK_HELD,
    ///       or a linked-worktree refusal
    CloseOperatorGate {
        /// Operator gate slug, e.g. `session-mac-mini`.
        slug: String,
        /// Path to search from when locating brain.toml. Defaults to the current directory.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Required. Asserts the operator confirmed the edge's `exit` artifact exists.
        #[arg(long = "exit-verified")]
        exit_verified: bool,
    },
    /// Normalize every stuttering operator/approval slug (D76) fleet-wide:
    /// `mev normalize-op-slugs [--write]`.
    ///
    /// A slug carrying a redundant `operator-` prefix (e.g.
    /// `operator-mac-mini-visit`) stutters when rendered as `OP.<slug>`. This
    /// command finds every such slug anywhere in the loaded corpus, groups ALL
    /// `operator`/`approval` `depends_on` edges carrying that exact slug —
    /// across every file, every repo — and renames every one of them to its
    /// normalized target in one atomic pass per slug. One slug can gate several
    /// blocks across several repos; renaming some of its edges but not others
    /// would split one gate into two, so a shared slug is always renamed
    /// everywhere at once or not at all.
    ///
    /// **Collision detection runs before any write.** The full rename plan
    /// (every distinct stuttering slug found, mapped to its normalized target)
    /// is computed first. If two distinct slugs would normalize to the same
    /// target — including a stuttering slug colliding with an untouched,
    /// already-existing non-stuttering slug — the ENTIRE run aborts with no
    /// writes at all, even for the non-colliding renames in the same corpus.
    /// Silently merging two distinct gates into one shared identity is worse
    /// than leaving both stuttering.
    ///
    /// Dry-run by default: without `--write`, prints the full computed plan
    /// (old slug, new slug, edge count, repos touched) and writes nothing.
    /// `--write` applies it under the same `<root>/.mev-emit.lock` advisory
    /// lock every other authored-state writer takes (E_EMIT_LOCK_HELD on
    /// contention), then re-runs `emit-state --write` on success so rendered
    /// boards (`OP.<slug>`) reflect the renamed slugs immediately. Refused the
    /// same way as its siblings when run from inside a linked git worktree.
    ///
    /// Exit codes:
    ///   0 — planned (dry-run) or applied cleanly
    ///   1 — a collision, a write failure, E_EMIT_LOCK_HELD, or a
    ///       linked-worktree refusal
    NormalizeOpSlugs {
        /// Path to search from when locating brain.toml. Defaults to the current directory.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Apply the rename. Without this the command prints the computed plan
        /// (which slugs rename to what, how many edges/files each touches)
        /// without touching any files.
        #[arg(long)]
        write: bool,
    },
    /// Approve a pending decision gate: `mev approve <slug> --digest <d>`.
    ///
    /// Removes every `depends_on` `{type:"approval", slug: <slug>}` entry across
    /// every loaded `state.json`, but only when `--digest` matches the stored
    /// `digest` on every matching edge. A shared slug is meant to carry one
    /// reviewed payload, so a mismatch on even one matching edge refuses the whole
    /// call rather than clearing the edges that did match.
    ///
    /// **Digest mismatch is not a quiet failure.** The passed digest not matching
    /// the stored digest means the payload changed since it was reviewed — the
    /// approval is void. Nothing is removed (the edge stays unmet and re-queues as
    /// a fresh decision) and mev raises a distinct `E_APPROVAL_DIGEST_MISMATCH`
    /// diagnostic (per D71) rather than silently re-queuing: a payload changing
    /// under an approval may be legitimate drift or a bug, and the moment the
    /// digests disagree is the only cheap moment to catch it.
    ///
    /// SLUG matching no approval edge in the loaded corpus is an error, not a
    /// silent no-op — almost always a typo.
    ///
    /// On a successful (matched-digest) approval, re-runs `emit-state --write` so
    /// `focus`/the boards agree with the cleared gate, under the same
    /// `<root>/.mev-emit.lock` advisory lock every other authored-state writer
    /// takes (E_EMIT_LOCK_HELD on contention; a stale lock from a dead pid is
    /// reclaimed automatically). Refused the same way as its siblings when run
    /// from inside a linked git worktree.
    ///
    /// Exit codes:
    ///   0 — every matching edge removed (digest verified) and emit-state re-run cleanly
    ///   1 — unknown slug, digest mismatch (alarmed), a write failure,
    ///       E_EMIT_LOCK_HELD, or a linked-worktree refusal
    Approve {
        /// Approval gate slug, e.g. `dev-to-cta-sweep`.
        slug: String,
        /// Digest of the exact payload reviewed; must match the edge's stored digest.
        #[arg(long)]
        digest: String,
        /// Path to search from when locating brain.toml. Defaults to the current directory.
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Reject a pending decision gate: `mev reject <slug>`.
    ///
    /// Removes every `depends_on` `{type:"approval", slug: <slug>}` entry across
    /// every loaded `state.json`, regardless of `digest` — a rejection ends the
    /// decision whether the reviewed payload is still current or not. The
    /// rejection is recorded via the write's diagnostic note, same mechanism as
    /// `close-operator-gate`.
    ///
    /// SLUG matching no approval edge in the loaded corpus is an error, not a
    /// silent no-op — almost always a typo.
    ///
    /// On success, re-runs `emit-state --write` so `focus`/the boards agree with
    /// the cleared gate, under the same `<root>/.mev-emit.lock` advisory lock
    /// every other authored-state writer takes. Refused the same way as its
    /// siblings when run from inside a linked git worktree.
    ///
    /// Exit codes:
    ///   0 — every matching edge removed and emit-state re-run cleanly
    ///   1 — unknown slug, a write failure, E_EMIT_LOCK_HELD, or a linked-worktree
    ///       refusal
    Reject {
        /// Approval gate slug, e.g. `dev-to-cta-sweep`.
        slug: String,
        /// Path to search from when locating brain.toml. Defaults to the current directory.
        #[arg(default_value = ".")]
        path: PathBuf,
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
    /// Emit every Attention-board item, across all four lanes (stale carryover, aging
    /// backlog, orphaned captures, stale distilled knowledge), as a JSON array of
    /// `EN.8.A`-compatible operator payloads for `engine-rs`'s operator queue
    /// (`MV.ticket.attention-queue-delivery` task 5).
    ///
    /// Reuses the identical corpus load + `effective_priorities` derivation
    /// `emit-state`'s attention-board planner uses, so the queue can never diverge
    /// from what `/attention` itself would show. Ordered hottest-first by
    /// `effective_priority`, tie-broken by age descending then `item_id`
    /// ascending — deterministic, so an unchanged corpus reproduces byte-identical
    /// output run to run.
    ///
    /// Two boundaries this command does not cross (see `docs/cli.md`): it derives
    /// and emits an artifact only — it never enqueues into `engine-core`'s operator
    /// queue, opens a notification channel, or writes `state.json`; and it emits
    /// the full ordered set — depth limiting is the queue's job (`EN.8.B`), not
    /// this command's.
    ///
    /// An empty board prints `[]` and exits 0 — an empty queue is not an error.
    AttentionQueue {
        /// Path to search from when locating brain.toml (walks up to find it).
        /// Defaults to the current directory.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Write the JSON array to this file instead of stdout.
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
    /// Fleet-wide, read-only sweep of every discovered `planning/state.json`'s
    /// `carryover[]` array (`MV.ticket.carryover-sweep-command`).
    ///
    /// Resolves `brain.toml`, discovers and loads every repo's `planning/state.json`, and
    /// evaluates each `carryover[]` entry's `clears_when` predicate where it is
    /// machine-checkable, sorting the fleet into three lanes:
    ///   cleared        — every extracted reference is satisfied; a recommendation to
    ///                     delete the entry, never an automatic deletion
    ///   actionable     — at least one extracted reference is unsatisfied; the unmet
    ///                     reference(s) are named
    ///   not-evaluable  — no reference could be extracted (prose predicate, or no
    ///                     `clears_when` at all)
    ///
    /// Evaluates prose block/path references, plus the four typed `clears_when`
    /// predicates: `block_closed`, `file_exists`, `file_contains`, and
    /// `command_exits_zero`. References combine conjunctively (AND) even when the
    /// prose says "or" — the safe failure direction. `command_exits_zero` is never
    /// executed unless `--allow-exec` is passed; without it, such entries report
    /// NotEvaluable rather than running anything. Never writes anything.
    ///
    /// The human summary also prints three cross-repo dedup sections
    /// (`MV.ticket.carryover-dedup-clusters`), each omitted when empty:
    ///   CLUSTERS                     — entries sharing an authored `finding_id`, grouped
    ///                                  one cluster per id. Members render with their own
    ///                                  per-repo priority side by side; divergent priorities
    ///                                  across repos are shown as-is, never reconciled into
    ///                                  one number.
    ///   SUGGESTED DUPLICATES         — heuristic, UNCONFIRMED candidate duplicate pairs
    ///                                  over entries with no `finding_id`, from a token-
    ///                                  overlap pass. Never auto-merged; a human confirms a
    ///                                  match by hand-authoring a shared `finding_id`.
    ///   SINGLE-REPO finding_id WARNINGS — a `finding_id` used in only one repo, usually a
    ///                                  typo that silently failed to group.
    ///
    /// Exit codes:
    ///   0 — sweep completed, regardless of how many entries land in any lane
    ///   1 — brain.toml not found/unreadable, an unknown --repo slug, or a serialization
    ///       error under --json
    Carryover {
        /// Path to search from when locating brain.toml (walks up to find it).
        /// Defaults to the current directory.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Restrict the sweep to one repo's carryover[] entries.
        #[arg(long)]
        repo: Option<String>,
        /// Emit the CarryoverReport as compact JSON instead of a human summary.
        #[arg(long)]
        json: bool,
        /// Opt in to executing `command_exits_zero` predicates. Off by
        /// default — without this flag, every `command_exits_zero` entry
        /// reports NotEvaluable (reason: execution-not-allowed) rather than
        /// running the command, regardless of what it would have exited.
        #[arg(long)]
        allow_exec: bool,
        /// Report a fleet-wide `carryover[]`/`reference[]` census instead of the
        /// per-entry sweep: total, per-container and per-class/per-kind counts,
        /// typed-predicate coverage, and inflow/outflow over `--window` days. The
        /// clear-rate statistic is scoped to `carryover[]` only — `reference[]`
        /// entries are structurally never clearable and are excluded from its
        /// denominator. Recommends only; never deletes or rewrites anything.
        #[arg(long)]
        audit: bool,
        /// Window, in days, `--audit`'s inflow/outflow figures are measured over.
        /// Ignored without `--audit`.
        #[arg(long, default_value_t = 30)]
        window: i64,
        /// Move every CLEARED-lane `carryover[]` entry into its owning repo's
        /// `planning/carryover-archive.jsonl` and remove it from `state.json`
        /// (`MV.ticket.carryover-dispose`). A disposal is a MOVE, never a
        /// delete. Independent of `--allow-exec`: passing `--dispose` never
        /// implies command execution — a `command_exits_zero` predicate that
        /// is NotEvaluable for lack of `--allow-exec` is never disposal-eligible.
        #[arg(long)]
        dispose: bool,
        /// Compute and print the identical disposal plan `--dispose` would
        /// act on, without writing either `state.json` or
        /// `carryover-archive.jsonl`. Only meaningful together with
        /// `--dispose`; passed alone, `mev carryover` reports the misuse and
        /// exits non-zero rather than silently ignoring it.
        #[arg(long)]
        dry_run: bool,
    },
    /// Scan the corpus for mechanically-detectable `carryover[]` findings instead of
    /// having an agent notice them by hand (`MV.ticket.graph-derived-carryover-findings`).
    ///
    /// Two deterministic detectors ship today:
    ///   unregistered-lane-block  — an id in some `lane-*.json`'s `blocks[]` with no
    ///                              matching `tracks[].blocks[].id` in its owning
    ///                              repo's `state.json`
    ///   referenced-path-absent  — a path named as a script or generator in a command
    ///                              or spec that resolves nowhere in the fleet
    ///
    /// Each finding carries a stable, content-derived `finding_id` (hashed over the
    /// detector class and the normalized subject only — never the repo or file it was
    /// found in), so the *same* finding filed independently by several repos
    /// correlates to one id and `mev carryover`'s existing clustering can group them.
    ///
    /// Reports by default; never writes anything without `--write`. Diagnostics a
    /// detector's disk-facing wrapper surfaced (a malformed lane record, a file that
    /// failed to read) print alongside the findings, never silently swallowed.
    ///
    /// Exit codes:
    ///   0 — the corpus is clean: no findings and no error-severity diagnostic
    ///   1 — at least one finding was reported, or an error-severity diagnostic was
    ///       surfaced, or brain.toml was not found/unreadable, or `--json`
    ///       serialization failed
    GraphFindings {
        /// Path to search from when locating brain.toml (walks up to find it).
        /// Defaults to the current directory.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Emit the GraphFindingsReport as compact JSON instead of a human summary.
        #[arg(long)]
        json: bool,
        /// Append each finding to its owning repo's `state.json` `carryover[]` as a
        /// typed entry carrying its `finding_id`. Not yet implemented — the flag is
        /// wired ahead of the write path (task 5 of this spec) so the surface stays
        /// stable; passing it errors clearly rather than silently doing nothing.
        #[arg(long)]
        write: bool,
    },
    /// Run the registry of named drift checks over facts kept in two places
    /// (`MV.ticket.conformance-check-registry`).
    ///
    /// Each registered check canonicalizes and digests both sides of a duplicated fact
    /// and reports divergence with the concrete set difference. Four checks ship today:
    ///   backlog-parity           — HQ planning/backlog.md ## Active + ## Promoted vs
    ///                               state.json backlog[]
    ///   epics-index-parity       — core/planning/epics/index.md vs the HQ epics[] registry
    ///   project-cache-watermark  — docs/projects/<project>.md synced_from vs the sub-repo's
    ///                               real planning/status.md timestamp (an adapter over
    ///                               `mev validate-brain --sync`)
    ///   toolchain-freshness      — the running mev binary's compiled-in build stamp vs its
    ///                               source tree's current HEAD
    ///
    /// Each check reports one of three statuses: pass (both sides match), drift (the
    /// sides diverge — see the findings for the concrete set difference), or
    /// not-evaluable (the check's inputs were absent; never a substitute for drift).
    ///
    /// Exit codes:
    ///   0 — every check reported pass or not-evaluable
    ///   1 — at least one check reported drift
    ///   1 — brain.toml not found/unreadable, an unknown --check name, or a serialization
    ///       error under --json
    Conformance {
        /// Path to search from when locating brain.toml (walks up to find it).
        /// Defaults to the current directory.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Run exactly one named check instead of the full registry.
        #[arg(long)]
        check: Option<String>,
        /// Emit the ConformanceReport as compact JSON instead of a human summary.
        #[arg(long)]
        json: bool,
    },
    /// Compile every path-dependent consumer's test targets against the working mev and
    /// report the true outcome per consumer (`ticket-consumer-compile-gate`).
    ///
    /// Discovers consumers the same way `mev conformance`'s `consumer-dependency-parity`
    /// check does (`brain::conformance::consumers::discover_mev_consumers` — never a second
    /// discovery implementation), then for each one spawns exactly
    /// `CARGO_TARGET_DIR=<fresh temp dir> cargo nextest run --no-run --locked --manifest-path
    /// <consumer>/Cargo.toml` and classifies the result. Every flag on that command is
    /// load-bearing: `--no-run` compiles test targets without executing them (the break class
    /// this exists for lives only in test fixtures, invisible to `cargo build`); `--locked`
    /// refuses to silently rewrite a `Cargo.lock` this repo does not own; the fresh
    /// `CARGO_TARGET_DIR` avoids contending with that consumer's own build lane.
    ///
    /// Four outcomes, each with a distinct operator action:
    ///   pass           — the consumer compiles clean against this mev; nothing to do.
    ///   broken         — a genuine type/API break; fix the named sites in that consumer repo.
    ///                    This is the only outcome that fails the run.
    ///   lockfile-stale — the consumer's Cargo.lock is stale, not a code break; refresh that
    ///                    consumer's lockfile. Reported prominently but does NOT fail the run.
    ///   skipped-dirty  — the consumer has uncommitted changes, so its result is not evidence
    ///                    about mev's change either way; commit or stash there and re-run.
    ///                    Does NOT fail the run.
    ///   not-evaluable  — the failure did not match a known signature (or the run's inputs
    ///                    could not be gathered at all, e.g. Cargo.lock moved despite
    ///                    --locked); reported with a reason, never guessed as broken. Does NOT
    ///                    fail the run.
    ///
    /// Exit codes:
    ///   0 — every consumer reported pass, lockfile-stale, skipped-dirty, or not-evaluable
    ///   1 — at least one consumer reported broken, brain.toml not found/unreadable, or
    ///       --consumer named a slug that is not a discovered consumer
    CheckConsumers {
        /// Path to search from when locating brain.toml (walks up to find it).
        /// Defaults to the current directory.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Run exactly one discovered consumer by slug instead of every consumer.
        #[arg(long)]
        consumer: Option<String>,
        /// Emit the per-consumer results as compact JSON instead of a human summary.
        #[arg(long)]
        json: bool,
    },
    /// Print the corpus-wide lane frontier (`MV.13.B`, Task 4) — read-only.
    ///
    /// Closure runs in mev itself, over the untruncated in-process block graph
    /// (`max_nodes: usize::MAX`) — never the HTTP export's truncated default (which
    /// defaults to `max_nodes=400` against 756 blocks). Any HTTP-side closure (e.g.
    /// bastion `BA.19.C`'s `/lanes` endpoint) MUST send `max_nodes=2000` and hard-fail
    /// on `truncated: true` rather than degrade; see `docs/cli.md` and
    /// `docs/architecture.md` for the full consumer contract. mev cannot gate that
    /// half itself — the evidence lives in bastion's own repo.
    ///
    /// Without `--json`: one line per frontier entry,
    /// `{roadmap}/{lane}#{segment} {repo}:{id} — startable | blocked by <reasons>`.
    /// With `--json`: the same shape `mev emit-state` writes to
    /// `planning/lane-frontier.json` (`derived_at`, `entries`, `gate_ranks`), printed
    /// to stdout rather than written to disk — this command never writes a file.
    ///
    /// Exit codes:
    ///   0 — frontier computed and printed
    ///   1 — brain.toml not found/unreadable, or the in-process graph somehow reported
    ///       `truncated: true` (should not happen at `max_nodes: usize::MAX`, but this
    ///       command refuses rather than silently degrading if it ever does)
    Frontier {
        /// Path to search from when locating brain.toml (walks up to find it).
        /// Defaults to the current directory.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Emit the frontier as JSON (the `lane-frontier.json` artifact shape) instead
        /// of one text line per entry.
        #[arg(long)]
        json: bool,
    },
    /// Compute six-state lane-segment availability plus lane-level unblock leverage
    /// over the corpus's frontier — `MV.13.C` Task 5.
    ///
    /// Read-only; never writes `planning/lane-availability.json` (that is `mev
    /// emit-state --write`'s job). Same untruncated-graph refusal as `mev frontier`:
    /// this command always builds the in-process block graph with
    /// `max_nodes: usize::MAX` and hard-fails rather than degrade if the export
    /// somehow reports `truncated: true`. See `docs/cli.md` and
    /// `docs/architecture.md` for the six states, their precedence, and the single
    /// lane-liveness source `HeldRepoBusy` reads from.
    ///
    /// Without `--json`: one line per segment, `{roadmap}/{lane}#{segment}
    /// {repo}:{head} — {availability} ({reason}) frees N lane(s)`. With `--json`:
    /// the same shape `mev emit-state` writes to `planning/lane-availability.json`
    /// (`derived_at`, `degraded`, `segments`), printed to stdout rather than written
    /// to disk — this command never writes a file.
    ///
    /// Exit codes:
    ///   0 — availability computed and printed
    ///   1 — brain.toml not found/unreadable, or the in-process graph somehow
    ///       reported `truncated: true`
    Lanes {
        /// Path to search from when locating brain.toml (walks up to find it).
        /// Defaults to the current directory.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Emit availability as JSON (the `lane-availability.json` artifact shape)
        /// instead of one text line per segment.
        #[arg(long)]
        json: bool,
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

/// Dispatch for `mev state-history <path> [--restore SEQ]`.
///
/// `path` names the *target file itself* (e.g. `planning/state.json`), not a brain
/// root to search from — every other subcommand's `path` walks up looking for
/// `brain.toml`; this one already knows exactly which file's history it wants. The
/// worktree guard and the advisory lock, both of which shell out to `git`/read
/// `brain.toml`, resolve from `path`'s *parent directory* instead — `git -C <file>`
/// is not meaningful.
fn run_state_history(path: &std::path::Path, restore: Option<u32>, json: bool) -> ExitCode {
    match restore {
        None => {
            // Read-only: list revisions, newest first. Never takes the lock.
            let mut revisions = match mev::brain::history::list_revisions(path) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::FAILURE;
                }
            };
            revisions.reverse(); // ascending -> newest first

            if json {
                match serde_json::to_string_pretty(&revisions) {
                    Ok(s) => println!("{s}"),
                    Err(err) => {
                        eprintln!("error serializing JSON: {err:#}");
                        return ExitCode::FAILURE;
                    }
                }
            } else if revisions.is_empty() {
                println!("no revisions recorded for {}", path.display());
            } else {
                for r in &revisions {
                    println!("{:>6}  {}  {} bytes", r.seq, r.recorded_at, r.bytes);
                }
            }
            ExitCode::SUCCESS
        }
        Some(seq) => {
            let dir = path.parent().unwrap_or_else(|| std::path::Path::new("."));

            if mev::brain::config::is_linked_worktree(dir) {
                eprintln!(
                    "error: refusing to run state-history --restore from inside a linked git worktree ({}) — restore resolves the advisory lock and brain.toml from the target file's own directory, not CWD, so restoring from a worktree would silently race the MAIN checkout's writers. Run `mev state-history --restore` from the main working tree instead.",
                    path.display()
                );
                return ExitCode::FAILURE;
            }

            let root = match mev::brain::config::find_brain_root(dir) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::FAILURE;
                }
            };

            // Advisory lock: --restore mutates a derived file and must not interleave
            // with a concurrent `emit-state --write` (or another restore).
            let _lock_guard = match mev::brain::lock::acquire_lock(
                &root,
                mev::brain::lock::DEFAULT_LOCK_TIMEOUT,
            ) {
                Ok(guard) => guard,
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
            };

            let revisions = match mev::brain::history::list_revisions(path) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::FAILURE;
                }
            };
            if revisions.is_empty() {
                println!("no revisions recorded for {}", path.display());
                return ExitCode::SUCCESS;
            }
            let min_seq = revisions.first().map(|r| r.seq).unwrap_or(0);
            let max_seq = revisions.last().map(|r| r.seq).unwrap_or(0);
            if !revisions.iter().any(|r| r.seq == seq) {
                eprintln!(
                    "error: no revision {seq} recorded for {}; valid range is {min_seq}..={max_seq}",
                    path.display()
                );
                return ExitCode::FAILURE;
            }

            let restored_content = match mev::brain::history::read_revision(path, seq) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::FAILURE;
                }
            };

            // First record the current on-disk content as a new revision, so a wrong
            // restore is itself undoable — a snapshot failure never blocks the
            // restore (history is a safety net, never a new failure mode).
            let pre_restore_revision = match std::fs::read(path) {
                Ok(current) => match mev::brain::history::record_revision(path, &current) {
                    Ok(rev) => Some(rev),
                    Err(e) => {
                        eprintln!(
                            "warning [W_HISTORY_FAILED]: failed to record pre-restore revision for {}: {e}",
                            path.display()
                        );
                        None
                    }
                },
                Err(_) => None, // nothing on disk yet to snapshot
            };

            if let Err(e) = mev::write_atomic(path, &restored_content) {
                eprintln!("error: failed to write {}: {e}", path.display());
                return ExitCode::FAILURE;
            }

            if json {
                let out = serde_json::json!({
                    "restored_seq": seq,
                    "path": path.display().to_string(),
                    "pre_restore_revision": pre_restore_revision,
                });
                match serde_json::to_string_pretty(&out) {
                    Ok(s) => println!("{s}"),
                    Err(err) => {
                        eprintln!("error serializing JSON: {err:#}");
                        return ExitCode::FAILURE;
                    }
                }
            } else {
                match &pre_restore_revision {
                    Some(rev) => println!(
                        "restored revision {seq} to {} (pre-restore content saved as revision {})",
                        path.display(),
                        rev.seq
                    ),
                    None => println!("restored revision {seq} to {}", path.display()),
                }
            }
            ExitCode::SUCCESS
        }
    }
}

/// Which epic-status command [`run_epic_status`] is dispatching for.
///
/// A CLI-layer selector, distinct from [`mev::brain::epics::EpicAction`]: that
/// type only ever means "cascade a status change onto member blocks" (its two
/// variants are `Defer`/`Resume`), and `complete-epic` deliberately does not
/// cascade — see `plan_complete_epic`'s doc comment. Keeping the non-cascading
/// case out of `EpicAction` means the planner side of the cascade/no-cascade
/// distinction cannot be blurred by this CLI-level enum growing a matching
/// variant later.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EpicOp {
    Defer,
    Resume,
    Complete,
}

/// Shared dispatch for `defer-epic` / `resume-epic` / `complete-epic` / `sync-epics`.
///
/// All four resolve the brain root, take the same advisory lock, and report in
/// the same shape as `emit-state` (per-diagnostic lines + a mode/count summary,
/// or a JSON envelope under `--json`) — this is the one place that plumbing
/// lives, so it cannot drift between the sibling commands. `complete-epic`
/// dispatches to [`mev::complete_epic`] instead of [`mev::epic_status`]; the
/// other three still go through `epic_status`.
fn run_epic_status(
    path: &std::path::Path,
    slug: Option<&str>,
    op: EpicOp,
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

    let label = match (slug, op) {
        (Some(_), EpicOp::Defer) => "defer-epic",
        (Some(_), EpicOp::Resume) => "resume-epic",
        (Some(_), EpicOp::Complete) => "complete-epic",
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

    let outcome = match op {
        EpicOp::Complete => {
            // slug is always Some for complete-epic — there is no "complete every
            // epic" analog of sync-epics (see tasks.md's Notes: completion stays
            // an explicit, named operator judgement, never a reconcile-all pass).
            let s = slug.expect("complete-epic always names a slug");
            mev::complete_epic(&root, s, write)
        }
        EpicOp::Defer => mev::epic_status(&root, slug, mev::brain::epics::EpicAction::Defer, write),
        EpicOp::Resume => {
            mev::epic_status(&root, slug, mev::brain::epics::EpicAction::Resume, write)
        }
    };

    match outcome {
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

/// Returns whether the block named by `key` (`repo:id`) currently carries an unmet
/// `operator` `depends_on` entry — the check behind `set-block-status`'s D71
/// operator gate.
///
/// `None` means "could not determine" (bad key shape, `brain.toml` not found, the
/// block not found, or a `state.json` failed to load) — callers must treat that as
/// "don't gate" and let the normal `set-block-status` path surface the real error
/// (`E_BLOCK_BAD_KEY` / `E_CONFIG_NOT_FOUND` / `E_BLOCK_NOT_FOUND` / etc.), never as
/// an implicit pass on the gate.
fn block_has_unmet_operator_gate(root: &std::path::Path, key: &str) -> Option<bool> {
    use mev::brain::config::find_brain_config;
    use mev::brain::state::{BlockedBy, discover_state_files, load_state};

    let (repo_slug, block_id) = key.split_once(':')?;
    let config = find_brain_config(root).ok()?;
    let (sources, _diags) = discover_state_files(root, &config);
    for src in &sources {
        if src.repo_slug != repo_slug {
            continue;
        }
        let Ok(file) = load_state(&src.abs_path) else {
            continue;
        };
        for track in &file.tracks {
            for block in &track.blocks {
                if block.id == block_id {
                    return Some(
                        block
                            .depends_on
                            .iter()
                            .any(|d| matches!(d, BlockedBy::Operator { .. })),
                    );
                }
            }
        }
    }
    None
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

/// Discover, load, and evaluate the fleet's `carryover[]` corpus the same way
/// [`mev::carryover_sweep`] does, but — unlike that driver — also capture
/// every individual `state.json` load failure as a `(repo_slug, error text)`
/// pair rather than silently skipping it. `mev carryover --dispose`'s guard
/// (3) needs that text: a repo whose sweep produced an evaluation error must
/// be reported and skipped, never silently treated as having zero cleared
/// entries (`compute_disposal_plan`'s `load_errors` parameter).
///
/// Duplicated from `load_and_evaluate_carryover_corpus`'s discovery/load/
/// evaluate steps rather than reusing that private function directly, since
/// this task's scope is `src/main.rs` only and that function does not thread
/// individual load errors through.
#[allow(clippy::type_complexity)]
fn load_and_evaluate_carryover_corpus_for_dispose(
    root: &std::path::Path,
    repo_filter: Option<&str>,
    allow_exec: bool,
) -> anyhow::Result<(
    Vec<(mev::brain::state::StateSource, mev::brain::state::StateFile)>,
    Vec<(String, String)>,
    mev::CarryoverReport,
)> {
    use mev::brain::config::find_brain_config;
    use mev::brain::state::{discover_state_files, load_state};

    let config = find_brain_config(root)
        .map_err(|e| anyhow::anyhow!("brain.toml not found or unreadable: {e}"))?;

    let (sources, _discovery_diags) = discover_state_files(root, &config);

    let mut loaded: Vec<(mev::brain::state::StateSource, mev::brain::state::StateFile)> =
        Vec::new();
    let mut load_errors: Vec<(String, String)> = Vec::new();
    for src in &sources {
        match load_state(&src.abs_path) {
            Ok(file) => loaded.push((src.clone(), file)),
            Err(e) => load_errors.push((src.repo_slug.clone(), e.to_string())),
        }
    }

    if let Some(slug) = repo_filter
        && !sources.iter().any(|s| s.repo_slug == slug)
    {
        let mut valid_slugs: Vec<&str> = sources.iter().map(|s| s.repo_slug.as_str()).collect();
        valid_slugs.sort_unstable();
        valid_slugs.dedup();
        return Err(anyhow::anyhow!(
            "unknown --repo slug '{slug}'; valid slugs: {}",
            valid_slugs.join(", ")
        ));
    }

    let mut status_map: std::collections::HashMap<String, Option<String>> =
        std::collections::HashMap::new();
    for (src, file) in &loaded {
        for track in &file.tracks {
            for block in &track.blocks {
                let key = format!("{}:{}", src.repo_slug, block.id);
                status_map.insert(key, block.status.clone());
            }
        }
    }

    let mut repo_paths: std::collections::HashMap<String, PathBuf> =
        std::collections::HashMap::new();
    for repo in &config.repos {
        let repo_root = if repo.repo_path == "." || repo.repo_path.is_empty() {
            root.to_path_buf()
        } else {
            root.join(&repo.repo_path)
        };
        repo_paths.insert(repo.slug.clone(), repo_root);
    }

    let today = chrono::Local::now()
        .date_naive()
        .format("%Y-%m-%d")
        .to_string();
    let report = mev::evaluate_carryover(
        &loaded,
        &status_map,
        root,
        &repo_paths,
        &today,
        &config.attention,
        repo_filter,
        allow_exec,
    );

    Ok((loaded, load_errors, report))
}

/// Drive `mev carryover --dispose` (and `--dispose --dry-run`, same code
/// path — see [`mev::brain::carryover::dispose_repo`]'s own `dry_run`
/// parameter): re-run the sweep with load errors captured, compute the
/// disposal plan (task 1), print each candidate's full text before it is
/// moved (constraint (5)), write (or simulate) the move (task 2), then print
/// the per-repo summary and commit pathspec (task 3, constraints (1) and
/// (6)).
///
/// Exit code follows [`mev::brain::carryover::DisposeRunReport::succeeded`]:
/// a repo the sweep never reached (`skipped`) is reported, not fatal: a repo
/// whose write itself failed partway through is.
fn run_carryover_dispose(
    root: &std::path::Path,
    repo_filter: Option<&str>,
    allow_exec: bool,
    dry_run: bool,
) -> ExitCode {
    use mev::brain::carryover::{
        compute_disposal_plan, render_dispose_preamble, render_dispose_summary, run_dispose,
    };

    let (loaded, load_errors, report) =
        match load_and_evaluate_carryover_corpus_for_dispose(root, repo_filter, allow_exec) {
            Ok(v) => v,
            Err(err) => {
                eprintln!("error: {err:#}");
                return ExitCode::FAILURE;
            }
        };

    let plan = compute_disposal_plan(&report, &loaded, &load_errors);

    let preamble = render_dispose_preamble(&plan);
    if !preamble.is_empty() {
        println!("{preamble}\n");
    }

    let today = chrono::Local::now()
        .date_naive()
        .format("%Y-%m-%d")
        .to_string();
    let dispose_report = run_dispose(&plan, &loaded, &today, dry_run);

    println!("{}", render_dispose_summary(&dispose_report));

    if dispose_report.succeeded() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Human-readable, lane-grouped summary for `mev carryover`'s default (non-`--json`) output.
fn print_carryover_report(report: &mev::CarryoverReport) {
    println!(
        "carryover sweep: {} total — {} cleared, {} actionable, {} not-evaluable",
        report.total, report.cleared, report.actionable, report.not_evaluable
    );

    for lane in [
        mev::CarryoverLane::Cleared,
        mev::CarryoverLane::Actionable,
        mev::CarryoverLane::NotEvaluable,
    ] {
        let entries: Vec<&mev::CarryoverVerdict> =
            report.entries.iter().filter(|e| e.lane == lane).collect();
        if entries.is_empty() {
            continue;
        }
        let lane_label = match lane {
            mev::CarryoverLane::Cleared => "CLEARED",
            mev::CarryoverLane::Actionable => "ACTIONABLE",
            mev::CarryoverLane::NotEvaluable => "NOT-EVALUABLE",
        };
        println!("\n{lane_label} ({}):", entries.len());
        for entry in entries {
            let age = match entry.age_days {
                Some(d) if entry.stale => format!(" ({d}d, stale)"),
                Some(d) => format!(" ({d}d)"),
                None => String::new(),
            };
            println!(
                "  {}:{} [{}]{age} — {}",
                entry.repo, entry.slug, entry.kind, entry.text
            );
            match lane {
                mev::CarryoverLane::Actionable => {
                    for r in &entry.refs {
                        match r {
                            mev::CarryoverRef::Block { key, satisfied } => {
                                if !satisfied {
                                    println!("      unmet: {key}");
                                }
                            }
                            mev::CarryoverRef::Path { path, satisfied } => {
                                if !satisfied {
                                    println!("      unmet: {path}");
                                }
                            }
                            mev::CarryoverRef::PathAbsent { path, satisfied } => {
                                if !satisfied {
                                    println!("      unmet: {path} still exists (expected removed)");
                                }
                            }
                            mev::CarryoverRef::UnresolvedBlock { key } => {
                                println!("      unresolvable: {key} (not found in loaded corpus)");
                            }
                            mev::CarryoverRef::FileContains {
                                path,
                                pattern,
                                satisfied,
                            } => {
                                if !satisfied {
                                    println!("      unmet: {path} does not contain '{pattern}'");
                                }
                            }
                            mev::CarryoverRef::CommandExitsZero { command, satisfied } => {
                                if !satisfied {
                                    println!("      unmet: `{command}` did not exit 0");
                                }
                            }
                        }
                    }
                }
                mev::CarryoverLane::NotEvaluable => {
                    let reason = match entry.reason {
                        Some(mev::NotEvaluableReason::Prose) => "prose",
                        Some(mev::NotEvaluableReason::NoPredicate) => "no-predicate",
                        Some(mev::NotEvaluableReason::AmbiguousReference) => "ambiguous-reference",
                        Some(mev::NotEvaluableReason::NoClosureVerb) => "no-closure-verb",
                        Some(mev::NotEvaluableReason::ExecutionNotAllowed) => {
                            "execution-not-allowed (rerun with --allow-exec)"
                        }
                        Some(mev::NotEvaluableReason::GateMentionNotCheckable) => {
                            "gate-mention-not-checkable (candidate for a typed command_exits_zero predicate)"
                        }
                        None => "unknown",
                    };
                    println!("      reason: {reason}");
                }
                mev::CarryoverLane::Cleared => {}
            }
        }
    }

    if !report.clusters.is_empty() {
        println!("\nCLUSTERS ({}):", report.clusters.len());
        for cluster in &report.clusters {
            println!("  {}", cluster.finding_id);
            for member in &cluster.members {
                let priority = match member.priority {
                    Some(p) => format!("P{p}"),
                    None => "P?".to_string(),
                };
                println!(
                    "    {}:{} [{priority}] — {}",
                    member.repo, member.slug, member.text
                );
            }
        }
    }

    if !report.suggestions.is_empty() {
        println!(
            "\nSUGGESTED DUPLICATES ({}) — UNCONFIRMED:",
            report.suggestions.len()
        );
        for s in &report.suggestions {
            println!(
                "  {}:{} ~ {}:{} (jaccard {:.2}, overlap {:.2})",
                s.a_repo, s.a_slug, s.b_repo, s.b_slug, s.jaccard, s.overlap
            );
        }
        println!(
            "  note: heuristic candidates only — never auto-merged. A human confirms a match by \
             hand-authoring a shared finding_id in both entries' state.json."
        );
    }

    if !report.single_repo_finding_ids.is_empty() {
        println!(
            "\nSINGLE-REPO finding_id WARNINGS ({}):",
            report.single_repo_finding_ids.len()
        );
        for id in &report.single_repo_finding_ids {
            println!("  {id}");
        }
        println!(
            "  note: a finding_id used in only one repo is usually a typo that silently failed \
             to group with its intended cross-repo match."
        );
    }
}

/// Human-readable, detector-class-grouped summary for `mev graph-findings`'s default
/// (non-`--json`) output. Modelled on [`print_carryover_report`]'s lane-grouped shape.
fn print_graph_findings_report(report: &mev::GraphFindingsReport, diagnostics: &[mev::Diagnostic]) {
    for d in diagnostics {
        print_diagnostic(d);
    }

    println!(
        "graph-findings: {} total — {} unregistered-lane-block, {} referenced-path-absent",
        report.total, report.unregistered_lane_block, report.referenced_path_absent
    );

    for class in [
        mev::DetectorClass::UnregisteredLaneBlock,
        mev::DetectorClass::ReferencedPathAbsent,
    ] {
        let rows: Vec<&mev::GraphFinding> = report
            .findings
            .iter()
            .filter(|f| f.detector == class)
            .collect();
        if rows.is_empty() {
            continue;
        }
        println!("\n{} ({}):", class.tag(), rows.len());
        for row in rows {
            println!("  {} [{}] — {}", row.repo, row.finding_id, row.message);
        }
    }
}

/// Human-readable summary for `mev carryover --audit`'s default (non-`--json`) output.
fn print_carryover_audit(audit: &mev::CarryoverAudit) {
    println!(
        "carryover audit: {} total — {} carryover[], {} reference[]",
        audit.total, audit.carryover_count, audit.reference_count
    );

    if !audit.per_kind.is_empty() {
        println!("\nCARRYOVER[] BY KIND:");
        for (kind, count) in &audit.per_kind {
            println!("  {kind}: {count}");
        }
    }

    if !audit.per_class.is_empty() {
        println!("\nREFERENCE[] BY CLASS:");
        for (class, count) in &audit.per_class {
            println!("  {class}: {count}");
        }
    }

    println!(
        "\ntyped clears_when predicates: {} / {} carryover[] entries",
        audit.typed_predicate_count, audit.carryover_count
    );
    println!(
        "clear rate (carryover[] only, reference[] excluded): {}/{} = {:.1}%",
        audit.cleared_total,
        audit.clearable_total,
        audit.clear_rate * 100.0
    );
    println!(
        "last {} days — inflow: {}, outflow: {}",
        audit.window_days, audit.inflow, audit.outflow
    );
}

/// Human-readable, one-block-per-consumer summary for `mev check-consumers`'s default
/// (non-`--json`) output. Names the outcome AND the operator's next action for it — the
/// distinction the whole ticket exists to keep loud (`Broken` vs `LockfileStale` vs
/// `SkippedDirty` are not interchangeable "it's red" states).
fn print_check_consumers_report(results: &[mev::consumers::ConsumerResult]) {
    use mev::consumers::ConsumerOutcome;

    for result in results {
        match &result.outcome {
            ConsumerOutcome::Pass => {
                println!("\n{} [PASS]", result.slug);
            }
            ConsumerOutcome::Broken { errors } => {
                println!("\n{} [BROKEN]", result.slug);
                for e in errors {
                    println!("    {e}");
                }
                println!(
                    "    next: fix the named sites above in the {} repo",
                    result.slug
                );
            }
            ConsumerOutcome::LockfileStale => {
                println!("\n{} [LOCKFILE-STALE]", result.slug);
                println!(
                    "    next: refresh {}'s Cargo.lock (bookkeeping, not a code break)",
                    result.slug
                );
            }
            ConsumerOutcome::SkippedDirty => {
                println!("\n{} [SKIPPED-DIRTY]", result.slug);
                println!(
                    "    next: commit or stash uncommitted work in {} and re-run",
                    result.slug
                );
            }
            ConsumerOutcome::NotEvaluable { reason } => {
                println!("\n{} [NOT-EVALUABLE]", result.slug);
                println!("    reason: {reason}");
            }
        }
    }
}

/// Human-readable, one-block-per-check summary for `mev conformance`'s default
/// (non-`--json`) output.
fn print_conformance_report(report: &mev::ConformanceReport) {
    for result in &report.results {
        let status_label = match result.outcome.status {
            mev::CheckStatus::Pass => "PASS",
            mev::CheckStatus::Drift => "DRIFT",
            mev::CheckStatus::NotEvaluable => "NOT-EVALUABLE",
        };
        println!(
            "\n{} [{status_label}] — {}",
            result.name, result.description
        );
        if let Some(left) = &result.outcome.left {
            println!(
                "  {} ({} items): {}",
                left.label,
                left.items.len(),
                left.digest
            );
        }
        if let Some(right) = &result.outcome.right {
            println!(
                "  {} ({} items): {}",
                right.label,
                right.items.len(),
                right.digest
            );
        }
        match result.outcome.status {
            mev::CheckStatus::Drift => {
                for finding in &result.outcome.findings {
                    println!("    {finding}");
                }
            }
            mev::CheckStatus::NotEvaluable => {
                if let Some(reason) = &result.outcome.reason {
                    println!("    reason: {reason}");
                }
            }
            mev::CheckStatus::Pass => {}
        }
    }

    println!(
        "\nconformance: {} check(s) — {} pass, {} drift, {} not-evaluable",
        report.results.len(),
        report.pass_count,
        report.drift_count,
        report.not_evaluable_count
    );
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    if cli.build_stamp {
        match serde_json::to_string(&mev::brain::conformance::toolchain::stamp_json()) {
            Ok(s) => println!("{s}"),
            Err(err) => {
                eprintln!("error serializing build stamp: {err:#}");
                return ExitCode::FAILURE;
            }
        }
        return ExitCode::SUCCESS;
    }

    let Some(command) = cli.command else {
        eprintln!("error: a subcommand is required (or pass --build-stamp)");
        return ExitCode::FAILURE;
    };

    match command {
        Command::Validate { path, blog, lint } => {
            // The positional's default is resolved here rather than in the derive so it can
            // depend on --blog: leaving the clap default off the blog case keeps the existing
            // learn-tree default (`../learn-ai/content/learn`) untouched when --blog is absent.
            let path = path.unwrap_or_else(|| {
                if blog {
                    PathBuf::from("../learn-ai/content/blog/published")
                } else {
                    PathBuf::from("../learn-ai/content/learn")
                }
            });
            let (result, consumer_label) = if blog {
                (mev::validate_blog(&path), "blog")
            } else if lint {
                (mev::validate_with_lint(&path), "learn-ai")
            } else {
                (mev::validate(&path), "learn-ai")
            };
            match result {
                Ok(report) => {
                    if cli.json {
                        let envelope = mev::JsonReport::new(consumer_label, &path, &report);
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
            }
        }
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
        Command::ValidateState { path } => match mev::validate_state(&path) {
            Ok(report) => {
                if cli.json {
                    let envelope = mev::JsonReport::new("state-file", &path, &report);
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
        Command::EmitState {
            path,
            write,
            scope,
            require_fresh,
        } => {
            // Convenience alias: --require-fresh sets the same env var `emit_state`
            // reads, rather than threading a new parameter through its signature (see
            // `mev::MEV_REQUIRE_FRESH_ENV`'s doc comment) — set before emit_state runs.
            if require_fresh {
                // SAFETY: single-threaded at this point in `main` — no other thread
                // reads or writes the process environment concurrently.
                unsafe {
                    std::env::set_var(mev::MEV_REQUIRE_FRESH_ENV, "1");
                }
            }
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
        Command::StateHistory { path, restore } => run_state_history(&path, restore, cli.json),
        Command::DeferEpic { slug, path, write } => {
            run_epic_status(&path, Some(&slug), EpicOp::Defer, write, cli.json)
        }
        Command::ResumeEpic { slug, path, write } => {
            run_epic_status(&path, Some(&slug), EpicOp::Resume, write, cli.json)
        }
        Command::CompleteEpic { slug, path, write } => {
            run_epic_status(&path, Some(&slug), EpicOp::Complete, write, cli.json)
        }
        Command::SyncEpics { path, write } => {
            run_epic_status(&path, None, EpicOp::Defer, write, cli.json)
        }
        Command::SetBlockStatus {
            key,
            status,
            path,
            write,
            force_operator_gate,
            scope,
        } => {
            // --force-operator-gate is the only override that starts a block with an
            // unmet operator edge, and per D71 it is human-only. Refuse it outright
            // when stdin is not a TTY, before touching anything else — this is
            // deliberately not gated on --write: passing the flag from a script or
            // an agent's non-interactive shell is exactly the failure mode this
            // closes, dry run or not.
            if force_operator_gate && !std::io::stdin().is_terminal() {
                eprintln!(
                    "error [E_FORCE_OPERATOR_GATE_NOT_TTY] refusing --force-operator-gate on \
                     non-interactive stdin — this is the only override that starts a block with \
                     an unmet operator edge, and it is human-only (D71); an agent may never pass \
                     it, and there is no priority threshold or other bypass."
                );
                return ExitCode::FAILURE;
            }
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
            // Operator gate (D71): a block carrying an unmet `operator` depends_on
            // entry cannot be started (moved to `in_progress`) without
            // --force-operator-gate — and that flag was already refused above if
            // stdin is not a TTY, so reaching here with it set means a human typed
            // it. No priority threshold or other condition bypasses this check.
            if write
                && status == "in_progress"
                && !force_operator_gate
                && let Some(true) = block_has_unmet_operator_gate(&root, &key)
            {
                eprintln!(
                    "error [E_BLOCK_OPERATOR_GATED] refusing to start '{key}': it carries an \
                     unmet operator depends_on edge. Pass --force-operator-gate (human-only, \
                     refused on non-TTY stdin) to override."
                );
                return ExitCode::FAILURE;
            }
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
            // Resolve --scope the same way Command::EmitState does: one resolution
            // path serves both verbs, and config.scope_dependencies() is the only
            // implementation of it in the diff.
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
            report_doc(
                "set-block-status",
                &root,
                write,
                cli.json,
                mev::set_block_status(&root, &key, &status, write, scope_deps.as_ref()),
            )
        }
        Command::CloseOperatorGate {
            slug,
            path,
            exit_verified,
        } => {
            // Refuse before reading or touching anything — the whole point of
            // --exit-verified is that mev never infers the exit condition itself.
            if !exit_verified {
                eprintln!(
                    "error [{}] refusing to close operator gate '{slug}' without --exit-verified \
                     — the exit artifact's existence is the operator's assertion, never mev's \
                     inference.",
                    mev::brain::operator::E_OPERATOR_GATE_NOT_VERIFIED
                );
                return ExitCode::FAILURE;
            }
            // Same worktree guard as emit-state: this chains into emit-state, which
            // resolves every repo's paths from brain.toml rather than CWD.
            if mev::brain::config::is_linked_worktree(&path) {
                eprintln!(
                    "error: refusing to write from inside a linked git worktree ({}) — \
                     close-operator-gate chains into emit-state, which resolves derived-file \
                     paths from brain.toml, not CWD, so this would regenerate the MAIN \
                     checkout's files. Run from the main working tree instead.",
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
            // Advisory lock, same contract as every other authored-state writer: this
            // always mutates (there is no dry-run mode), so the lock is always taken.
            // Released via Drop on every exit path below.
            let _lock_guard = match mev::brain::lock::acquire_lock(
                &root,
                mev::brain::lock::DEFAULT_LOCK_TIMEOUT,
            ) {
                Ok(guard) => guard,
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
            };
            report_doc(
                "close-operator-gate",
                &root,
                true,
                cli.json,
                mev::close_operator_gate(&root, &slug, exit_verified),
            )
        }
        Command::NormalizeOpSlugs { path, write } => {
            // Same worktree guard as set-block-status/emit-state: a --write here
            // chains into emit-state, which resolves every repo's paths from
            // brain.toml rather than CWD.
            if write && mev::brain::config::is_linked_worktree(&path) {
                eprintln!(
                    "error: refusing to write from inside a linked git worktree ({}) — \
                     normalize-op-slugs chains into emit-state, which resolves derived-file \
                     paths from brain.toml, not CWD, so this would regenerate the MAIN \
                     checkout's files. Run from the main working tree instead.",
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
            // Advisory lock, same contract as set-block-status: only --write
            // mutates the corpus, so only --write needs mutual exclusion.
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
                "normalize-op-slugs",
                &root,
                write,
                cli.json,
                mev::normalize_op_slugs(&root, write),
            )
        }
        Command::Approve { slug, digest, path } => {
            // Same worktree guard as close-operator-gate/emit-state: this chains
            // into emit-state, which resolves every repo's paths from brain.toml
            // rather than CWD.
            if mev::brain::config::is_linked_worktree(&path) {
                eprintln!(
                    "error: refusing to write from inside a linked git worktree ({}) — approve \
                     chains into emit-state, which resolves derived-file paths from brain.toml, \
                     not CWD, so this would regenerate the MAIN checkout's files. Run from the \
                     main working tree instead.",
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
            // Advisory lock, same contract as every other authored-state writer.
            let _lock_guard = match mev::brain::lock::acquire_lock(
                &root,
                mev::brain::lock::DEFAULT_LOCK_TIMEOUT,
            ) {
                Ok(guard) => guard,
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
            };
            report_doc(
                "approve",
                &root,
                true,
                cli.json,
                mev::approve(&root, &slug, &digest),
            )
        }
        Command::Reject { slug, path } => {
            // Same worktree guard as close-operator-gate/emit-state.
            if mev::brain::config::is_linked_worktree(&path) {
                eprintln!(
                    "error: refusing to write from inside a linked git worktree ({}) — reject \
                     chains into emit-state, which resolves derived-file paths from brain.toml, \
                     not CWD, so this would regenerate the MAIN checkout's files. Run from the \
                     main working tree instead.",
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
            let _lock_guard = match mev::brain::lock::acquire_lock(
                &root,
                mev::brain::lock::DEFAULT_LOCK_TIMEOUT,
            ) {
                Ok(guard) => guard,
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
            };
            report_doc("reject", &root, true, cli.json, mev::reject(&root, &slug))
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
        Command::Frontier { path, json } => {
            let root = match mev::brain::config::find_brain_root(&path) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::FAILURE;
                }
            };

            match mev::frontier_brain(&root) {
                Ok(frontier) => {
                    if json {
                        let artifact = mev::brain::frontier::build_frontier_artifact(frontier);
                        match serde_json::to_string_pretty(&artifact) {
                            Ok(s) => {
                                println!("{s}");
                                ExitCode::SUCCESS
                            }
                            Err(err) => {
                                eprintln!("error serializing frontier: {err:#}");
                                ExitCode::FAILURE
                            }
                        }
                    } else {
                        let text = mev::brain::frontier::render_frontier_text(&frontier);
                        if !text.is_empty() {
                            println!("{text}");
                        }
                        ExitCode::SUCCESS
                    }
                }
                Err(err) => {
                    eprintln!("error: {err:#}");
                    ExitCode::FAILURE
                }
            }
        }
        Command::Lanes { path, json } => {
            let root = match mev::brain::config::find_brain_root(&path) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::FAILURE;
                }
            };

            match mev::lanes_brain(&root) {
                Ok(artifact) => {
                    if json {
                        match serde_json::to_string_pretty(&artifact) {
                            Ok(s) => {
                                println!("{s}");
                                ExitCode::SUCCESS
                            }
                            Err(err) => {
                                eprintln!("error serializing availability: {err:#}");
                                ExitCode::FAILURE
                            }
                        }
                    } else {
                        let text = mev::brain::availability::render_availability_text(&artifact);
                        if !text.is_empty() {
                            println!("{text}");
                        }
                        ExitCode::SUCCESS
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
        Command::Carryover {
            path,
            repo,
            json,
            allow_exec,
            audit,
            window,
            dispose,
            dry_run,
        } => {
            let root = match mev::brain::config::find_brain_root(&path) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::FAILURE;
                }
            };
            if dry_run && !dispose {
                eprintln!(
                    "error: --dry-run has no effect without --dispose; pass --dispose --dry-run"
                );
                return ExitCode::FAILURE;
            }
            if dispose {
                run_carryover_dispose(&root, repo.as_deref(), allow_exec, dry_run)
            } else if audit {
                match mev::carryover_audit(&root, repo.as_deref(), allow_exec, window) {
                    Ok((_report, audit)) => {
                        if json || cli.json {
                            match serde_json::to_string(&audit) {
                                Ok(s) => {
                                    println!("{s}");
                                    ExitCode::SUCCESS
                                }
                                Err(err) => {
                                    eprintln!("error serializing carryover audit: {err:#}");
                                    ExitCode::FAILURE
                                }
                            }
                        } else {
                            print_carryover_audit(&audit);
                            ExitCode::SUCCESS
                        }
                    }
                    Err(err) => {
                        eprintln!("error: {err:#}");
                        ExitCode::FAILURE
                    }
                }
            } else {
                match mev::carryover_sweep(&root, repo.as_deref(), allow_exec) {
                    Ok(report) => {
                        if json || cli.json {
                            match serde_json::to_string(&report) {
                                Ok(s) => {
                                    println!("{s}");
                                    ExitCode::SUCCESS
                                }
                                Err(err) => {
                                    eprintln!("error serializing carryover report: {err:#}");
                                    ExitCode::FAILURE
                                }
                            }
                        } else {
                            print_carryover_report(&report);
                            ExitCode::SUCCESS
                        }
                    }
                    Err(err) => {
                        eprintln!("error: {err:#}");
                        ExitCode::FAILURE
                    }
                }
            }
        }
        Command::GraphFindings { path, json, write } => {
            if write {
                eprintln!(
                    "error: --write is not yet implemented for graph-findings (lands in \
                     MV.ticket.graph-derived-carryover-findings task 5); rerun without --write"
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
            match mev::graph_findings_report(&root) {
                Ok((report, diagnostics)) => {
                    let has_error_diag = diagnostics.iter().any(|d| d.severity == Severity::Error);
                    if json || cli.json {
                        match serde_json::to_string(&report) {
                            Ok(s) => {
                                println!("{s}");
                            }
                            Err(err) => {
                                eprintln!("error serializing graph-findings report: {err:#}");
                                return ExitCode::FAILURE;
                            }
                        }
                    } else {
                        print_graph_findings_report(&report, &diagnostics);
                    }
                    if report.total > 0 || has_error_diag {
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
        Command::Conformance { path, check, json } => {
            let root = match mev::brain::config::find_brain_root(&path) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::FAILURE;
                }
            };
            match mev::conformance(&root, check.as_deref()) {
                Ok(report) => {
                    let drift = report.drift_count > 0;
                    if json || cli.json {
                        match serde_json::to_string(&report) {
                            Ok(s) => {
                                println!("{s}");
                            }
                            Err(err) => {
                                eprintln!("error serializing conformance report: {err:#}");
                                return ExitCode::FAILURE;
                            }
                        }
                    } else {
                        print_conformance_report(&report);
                    }
                    if drift {
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
        Command::CheckConsumers {
            path,
            consumer,
            json,
        } => {
            let root = match mev::brain::config::find_brain_root(&path) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::FAILURE;
                }
            };
            match mev::check_consumers(&root, consumer.as_deref()) {
                Ok(results) => {
                    let broken = results.iter().any(|r| {
                        matches!(r.outcome, mev::consumers::ConsumerOutcome::Broken { .. })
                    });
                    if json || cli.json {
                        match serde_json::to_string(&results) {
                            Ok(s) => println!("{s}"),
                            Err(err) => {
                                eprintln!("error serializing check-consumers results: {err:#}");
                                return ExitCode::FAILURE;
                            }
                        }
                    } else {
                        print_check_consumers_report(&results);
                    }
                    if broken {
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
        Command::AttentionQueue { path, out } => {
            let root = match mev::brain::config::find_brain_root(&path) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::FAILURE;
                }
            };
            match mev::attention_queue(&root) {
                Ok(json) => {
                    if let Some(out_path) = out {
                        if let Err(err) = std::fs::write(&out_path, format!("{json}\n")) {
                            eprintln!("error: could not write {}: {err:#}", out_path.display());
                            return ExitCode::FAILURE;
                        }
                    } else {
                        println!("{json}");
                    }
                    ExitCode::SUCCESS
                }
                Err(err) => {
                    eprintln!("error: {err:#}");
                    ExitCode::FAILURE
                }
            }
        }
    }
}
