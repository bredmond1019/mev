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
    match cli.command {
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
        Command::Carryover {
            path,
            repo,
            json,
            allow_exec,
        } => {
            let root = match mev::brain::config::find_brain_root(&path) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::FAILURE;
                }
            };
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
