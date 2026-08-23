//! Mechanically-detectable carryover findings — `mev graph-findings` (Phase Jynx,
//! `MV.ticket.graph-derived-carryover-findings`).
//!
//! A whole class of `carryover[]` entries is deterministically derivable from the
//! corpus rather than found by an agent reading files: a lane file naming a block
//! no `state.json` registers, or a doc naming a script that exists nowhere. This
//! module owns the report model ([`GraphFinding`], [`DetectorClass`],
//! [`GraphFindingsReport`]) and the stable, content-derived [`finding_id`] shared by
//! every detector, so the *same* finding filed independently by several repos
//! correlates to one id (task 1). The detectors themselves —
//! `unregistered-lane-block` (task 2) and `referenced-path-absent` (task 3) — are
//! layered on top of this module in follow-on tasks of the same file.
//!
//! Modelled on this crate's two established report shapes —
//! [`crate::brain::block_graph::BlockGraphExport`] and
//! [`crate::brain::carryover::CarryoverReport`] — rather than inventing a third
//! convention: a header/summary struct, a flat `Vec` of typed rows, and per-class
//! counts alongside the total.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest as _, Sha256};

use okf_core::ClearsWhenPredicate;

use crate::Diagnostic;
use crate::brain::config::BrainConfig;
use crate::brain::lane_segments::{LaneFile, discover_lane_files};
use crate::brain::state::{StateFile, StateSource, discover_state_files, load_state};

// ---------------------------------------------------------------------------
// Detector classes
// ---------------------------------------------------------------------------

/// Which deterministic detector produced a [`GraphFinding`].
///
/// Two classes ship in this ticket. Adding a third means adding a variant here,
/// a `tag()` arm, a counter field on [`GraphFindingsReport`], and — because
/// `tag()` feeds [`finding_id`] — every existing `finding_id` stays stable
/// (the tag strings of the existing variants must never change).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DetectorClass {
    /// A block id named in some `lane-*.json`'s `blocks[]` has no matching
    /// `tracks[].blocks[].id` in its owning repo's `state.json`.
    UnregisteredLaneBlock,
    /// A path named as a script or generator in a command or spec resolves
    /// nowhere in the fleet.
    ReferencedPathAbsent,
}

impl DetectorClass {
    /// Stable, `kebab-case` identifier for this class — used both as the
    /// human-readable label and as the first component hashed into
    /// [`finding_id`]. **Never rename an existing arm's string**: doing so
    /// changes every `finding_id` already written to a live `state.json`.
    #[must_use]
    pub fn tag(self) -> &'static str {
        match self {
            DetectorClass::UnregisteredLaneBlock => "unregistered-lane-block",
            DetectorClass::ReferencedPathAbsent => "referenced-path-absent",
        }
    }
}

// ---------------------------------------------------------------------------
// finding_id
// ---------------------------------------------------------------------------

/// Derive the stable, content-derived `finding_id` for one finding.
///
/// Hashed over `(detector.tag(), subject)` and **nothing else** — never the
/// owning repo, the file path the finding was found in, a timestamp, or an
/// index. That is the entire point (per the block record): the same finding
/// filed independently from three different repos must produce the same id,
/// so `mev carryover`'s existing clustering can correlate them. `subject`
/// must already be normalized by the caller (see
/// [`normalize_referenced_path`] for the `ReferencedPathAbsent` case) —
/// this function does no normalization of its own, so two differently
/// spelled subjects for the same real-world thing will *not* collide unless
/// the caller normalizes first.
///
/// `sha2` is already a direct dependency (used by
/// [`crate::brain::attention_payload::item_id_for`]); reused here rather
/// than adding a second hasher.
#[must_use]
pub fn finding_id(detector: DetectorClass, subject: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(detector.tag().as_bytes());
    hasher.update(b"\0");
    hasher.update(subject.as_bytes());
    format!("{:x}", hasher.finalize())
}

// ---------------------------------------------------------------------------
// Path normalization (contract for the ReferencedPathAbsent subject)
// ---------------------------------------------------------------------------

/// Normalize a referenced path into the stable subject fed to [`finding_id`]
/// for the `referenced-path-absent` class.
///
/// Collapses a bare relative reference, a `./`-prefixed one, and one
/// prefixed with a single leading repo/tier directory to the same subject —
/// exactly the `render-spec.py` case the block record requires to
/// correlate across repos: `scripts/render_spec.py`,
/// `./scripts/render_spec.py`, and `base-template/scripts/render_spec.py`
/// all normalize to `scripts/render_spec.py`.
///
/// Rule: strip a leading `./`, split on `/`, and keep at most the last two
/// non-empty components (parent directory + file name). A path of one or
/// two components is returned unchanged (aside from the `./` strip); a
/// deeper path has any leading repo/tier/nesting prefix dropped. This is
/// deliberately coarse — `parent/name.ext` is specific enough to avoid
/// collapsing genuinely distinct scripts that merely share a filename in
/// different parent directories (e.g. `scripts/build.sh` vs
/// `hooks/build.sh` stay distinct), while dropping exactly the kind of
/// single leading repo-name prefix that made the same script look like
/// three different subjects across three repos.
#[must_use]
pub fn normalize_referenced_path(path: &str) -> String {
    let trimmed = path.strip_prefix("./").unwrap_or(path);
    let components: Vec<&str> = trimmed.split('/').filter(|c| !c.is_empty()).collect();
    if components.len() <= 2 {
        components.join("/")
    } else {
        components[components.len() - 2..].join("/")
    }
}

// ---------------------------------------------------------------------------
// Report model
// ---------------------------------------------------------------------------

/// One deterministically-detected finding.
#[derive(Debug, Clone, Serialize)]
pub struct GraphFinding {
    /// Which detector produced this row.
    pub detector: DetectorClass,
    /// The repo the finding is scoped to (the owning repo whose `state.json`
    /// would receive the `carryover[]` entry on `--write`).
    pub repo: String,
    /// The normalized subject hashed into `finding_id` — see
    /// [`normalize_referenced_path`] for the `ReferencedPathAbsent` case, and
    /// the `{repo}:{id}` key for `UnregisteredLaneBlock` (task 2).
    pub subject: String,
    /// Human-readable explanation, self-contained enough to act on without
    /// opening the source file.
    pub message: String,
    /// Stable, content-derived id from [`finding_id`] — identical across
    /// every repo independently reporting the same `(detector, subject)`.
    pub finding_id: String,
    /// Typed condition under which this finding — and the `carryover[]`
    /// entry `--write` derives from it via
    /// [`crate::brain::carryover::carryover_entry_for_finding`] — should be
    /// deleted (`MV.ticket.graph-findings-path-resolution` task 3). Always
    /// `Some`: every finding this module emits is machine-detected from a
    /// disagreement between two data sources, and that disagreement is
    /// itself machine-recheckable, so there is no detector class here for
    /// which `None` would be honest.
    ///
    /// **Reconciliation with `mev carryover`'s evaluator**
    /// (`path_ref_satisfied` / `resolve_existing_path` in
    /// `src/brain/carryover.rs`): the evaluator only ever tries two roots —
    /// the brain root, then the owning repo's `repo_path` — never
    /// `base-template` or a synced command's owning repo. Task 1's detector
    /// search order is wider than that (four roots). Each predicate below is
    /// spelled so the evaluator's narrower two-root check lines up with the
    /// SAME verdict the detector reached for roots (a)/(b) specifically; a
    /// finding that resolved only via root (c)/(d) (`base-template` or a
    /// synced command's owner) is never emitted in the first place — see
    /// [`referenced_path_absent_findings`] — so this narrower reconciliation
    /// is never asked to reproduce a (c)/(d)-only verdict, only to notice a
    /// (a)/(b) repair.
    pub clears_when: Option<ClearsWhenPredicate>,
}

/// The full `mev graph-findings` report.
#[derive(Debug, Clone, Default, Serialize)]
pub struct GraphFindingsReport {
    /// Total findings across every detector class.
    pub total: usize,
    /// Count of [`DetectorClass::UnregisteredLaneBlock`] findings.
    pub unregistered_lane_block: usize,
    /// Count of [`DetectorClass::ReferencedPathAbsent`] findings.
    pub referenced_path_absent: usize,
    /// Every finding, in detection order.
    pub findings: Vec<GraphFinding>,
}

impl GraphFindingsReport {
    /// Build a report from a flat list of findings, deriving `total` and the
    /// per-class counts from the rows themselves so the counts can never
    /// drift out of sync with `findings`.
    #[must_use]
    pub fn from_findings(findings: Vec<GraphFinding>) -> Self {
        let mut report = GraphFindingsReport {
            total: findings.len(),
            ..GraphFindingsReport::default()
        };
        for finding in &findings {
            match finding.detector {
                DetectorClass::UnregisteredLaneBlock => report.unregistered_lane_block += 1,
                DetectorClass::ReferencedPathAbsent => report.referenced_path_absent += 1,
            }
        }
        report.findings = findings;
        report
    }
}

// ---------------------------------------------------------------------------
// Detector 1: unregistered-lane-block
// ---------------------------------------------------------------------------

/// Detector 1 — `unregistered-lane-block`, over already-discovered/loaded corpus
/// data. Pure with respect to disk: callers do the I/O (see
/// [`detect_unregistered_lane_blocks`] for the disk-facing wrapper task 4's CLI
/// subcommand uses), which keeps this half unit-testable without a fixture
/// directory per case.
///
/// For every `blocks[]` entry in every discovered lane record, reports a finding
/// when the entry's OWN authored `repo` (never the lane's or the lane file's
/// location — a lane is not single-repo in this corpus, per
/// [`crate::brain::lane_segments::LaneBlockRef::repo`]) has no `state.json` with a
/// matching `tracks[].blocks[].id`. An id registered under a *different* repo than
/// the one the lane entry names still produces a finding — registration is keyed
/// on `(repo, id)`, not `id` alone.
///
/// `lane_diags` (the diagnostics [`discover_lane_files`] returned alongside
/// `lane_files`) are carried through verbatim into the returned diagnostics — a
/// lane record that failed to parse must surface as an error here too, never
/// silently contribute zero findings (standing rule 11: a clean-looking empty
/// result from a detector that could not read its input is the exact failure mode
/// this exists to avoid).
///
/// `config` supplies each repo's `repo_path` (brain-root-relative) so the
/// emitted `clears_when` predicate (task 3) can name a brain-root-relative
/// `<repo_path>/planning/state.json` — the same file this detector already
/// reads for `registered`, so `mev carryover`'s evaluator resolving it later
/// checks the identical file. When `block_ref.repo` has no `[[repos]]` entry
/// in `config` (a lane naming a repo that is not even registered — a
/// different, worse problem than this detector targets), the repo slug
/// itself is used as a best-effort `repo_path` fallback rather than omitting
/// the predicate: every finding this module emits carries a typed
/// `clears_when` by construction (see [`GraphFinding::clears_when`]).
#[must_use]
pub fn unregistered_lane_block_findings(
    lane_files: &[LaneFile],
    lane_diags: &[Diagnostic],
    state_files: &[(StateSource, StateFile)],
    config: &BrainConfig,
) -> (Vec<GraphFinding>, Vec<Diagnostic>) {
    let mut registered: HashSet<(&str, &str)> = HashSet::new();
    for (src, file) in state_files {
        for track in &file.tracks {
            for block in &track.blocks {
                registered.insert((src.repo_slug.as_str(), block.id.as_str()));
            }
        }
    }

    let mut findings = Vec::new();
    for lane_file in lane_files {
        for block_ref in &lane_file.blocks {
            if registered.contains(&(block_ref.repo.as_str(), block_ref.id.as_str())) {
                continue;
            }
            let subject = format!("{}:{}", block_ref.repo, block_ref.id);
            let message = format!(
                "lane '{}' (roadmap '{}', {}) names block '{}' owned by repo '{}', which has \
                 no matching tracks[].blocks[].id in {}'s planning/state.json",
                lane_file.lane,
                lane_file.roadmap,
                lane_file.path.display(),
                block_ref.id,
                block_ref.repo,
                block_ref.repo,
            );
            let repo_path = config
                .repos
                .iter()
                .find(|r| r.slug == block_ref.repo)
                .map(|r| r.repo_path.clone())
                .unwrap_or_else(|| block_ref.repo.clone());
            let clears_when = ClearsWhenPredicate::FileContains {
                path: format!("{repo_path}/planning/state.json"),
                pattern: block_ref.id.clone(),
                note: Some(format!(
                    "clears when block '{}' appears in {}'s planning/state.json \
                     tracks[].blocks[].id",
                    block_ref.id, block_ref.repo,
                )),
            };
            findings.push(GraphFinding {
                detector: DetectorClass::UnregisteredLaneBlock,
                repo: block_ref.repo.clone(),
                subject: subject.clone(),
                message,
                finding_id: finding_id(DetectorClass::UnregisteredLaneBlock, &subject),
                clears_when: Some(clears_when),
            });
        }
    }

    (findings, lane_diags.to_vec())
}

/// Disk-facing wrapper for [`unregistered_lane_block_findings`]: discovers every
/// lane record and every `planning/state.json` under `root` and reduces them
/// through the pure detector above. This is what task 4's `mev graph-findings`
/// subcommand calls; a `state.json` that fails to load is skipped (matching
/// [`crate::block_graph_brain`]'s posture — an individual malformed file is not
/// fatal to the run), while a lane record that fails to parse is surfaced as an
/// error diagnostic via `lane_diags`, never silently dropped.
#[must_use]
pub fn detect_unregistered_lane_blocks(
    root: &Path,
    config: &BrainConfig,
) -> (Vec<GraphFinding>, Vec<Diagnostic>) {
    let (lane_files, lane_diags) = discover_lane_files(root);
    let (sources, _state_discovery_diags) = discover_state_files(root, config);

    let mut loaded: Vec<(StateSource, StateFile)> = Vec::new();
    for src in &sources {
        if let Ok(file) = load_state(&src.abs_path) {
            loaded.push((src.clone(), file));
        }
    }

    unregistered_lane_block_findings(&lane_files, &lane_diags, &loaded, config)
}

// ---------------------------------------------------------------------------
// Detector 2: referenced-path-absent
// ---------------------------------------------------------------------------
//
// ## Scope contract (read this before extending the matcher)
//
// **Files scanned**, per repo (each `[[repos]]` entry in `brain.toml`):
// - every `.md` file under `<repo>/.claude/commands/` — a "command"
// - every `.json` file under `<repo>/planning/blocks/` — a "spec" (block record)
//
// No other markdown (READMEs, plans, decisions, prose docs generally) is
// scanned, and no other JSON (`state.json`, `tasks.json`, lane files) is
// scanned. Commands and block records are where a script/generator gets
// *invoked*, not merely mentioned in passing prose — the block record's own
// example, `render-spec.py`, is exactly this: a generator a command shells
// out to and a spec names as a `validation_commands`/interface detail. An
// over-broad matcher that treated every path-like string in prose as a
// reference would bury these findings under noise from narrative docs that
// merely *discuss* a script (D-numbers, block IDs, and prose paths in a
// `notes` field are not references to resolve).
//
// **What shape counts as "named as a script or generator"**: a
// whitespace/punctuation-delimited run of path characters (ASCII
// alphanumerics, `.`, `_`, `-`, `/`) that contains at least one `/` (so it
// reads as a *path*, not a bare word) and ends in `.py` or `.sh` — the two
// extensions the fleet's own `scripts/` and command-invoked generators
// actually use (see `planning/harness.json`'s shell checks and the
// `render_spec.py` case named in the block record). Deliberately excluded:
// bare filenames with no directory component (too easy to collide with an
// unrelated word), URLs (`://` is not a path character so a URL token never
// reaches the extension check), and every other extension (`.rs`, `.md`,
// `.json`, …) — those are either source code (not "a script a command
// runs") or not script/generator shapes at all. Extending the extension set
// is a deliberate scope change, not a bug fix; do it by editing
// [`is_script_extension`] and adding a test, not by relaxing the character
// class.
//
// **Resolution follows symlinks.** Every `planning/` in this corpus is a
// symlink into a `_planning/` vault (D46). [`std::path::Path::exists`]
// resolves symlinks at *every* path component (it is backed by `stat`, not
// `lstat`), so `repo_root.join(&raw_path).exists()` already does the right
// thing for a referenced path that only exists through a `planning/`
// symlink — no special-casing needed in the existence check itself. The
// trap is on the *walk* side instead: [`walkdir::WalkDir`] does not descend
// into an interior symlinked directory unless `.follow_links(true)` is set,
// so [`detect_referenced_path_absent`] sets it explicitly when walking for
// command/spec files to scan — otherwise a repo whose `planning/blocks/`
// is itself a symlink target would never have its specs discovered at all,
// which (per standing rule 11) would look like a clean empty result for
// exactly the wrong reason.
//
// **Resolution is fleet-wide, not repo-local (MV.ticket.graph-findings-path-resolution
// task 1).** A shared fleet script referenced from a synced `.claude/commands/*.md`
// file lives once, in the repo that owns the original, but the command that
// references it is copied into every repo it was synced to — so a repo-local-only
// check reports the same real file "absent" in every one of those copies. A
// referenced path is resolved against, in order, the FIRST match wins:
//
//   (a) `repo:<repo>`    — the referencing repo's own root (the original,
//                          only check before this ticket).
//   (b) `brain-root`     — the fleet HQ root (`agentic-portfolio/`), for a
//                          path referenced relative to the brain rather than
//                          any one repo.
//   (c) `base-template`  — `base-template/`'s own root, the source every
//                          `.claude/commands/*.md` file in the fleet is
//                          synced FROM (D54-family sync), so a shared
//                          fleet script committed there resolves for every
//                          repo it was synced to.
//   (d) `owner:base-template` — added only when the referencing file is a
//                          synced command: a `.claude/commands/*.md` whose
//                          basename also exists under
//                          `base-template/.claude/commands/`. Resolved
//                          through [`BrainConfig`]'s `base-template`
//                          `[[repos]]` entry rather than a hardcoded path,
//                          so a future repo-path change to that entry is
//                          picked up automatically. In today's fleet this
//                          shares (c)'s base path — it exists as a distinct,
//                          separately-labeled root because the *reason* a
//                          synced command's reference resolves is "this is
//                          the file's origin template", which is a
//                          different fact from (c)'s "base-template is
//                          always in the search path regardless of file
//                          type", and the two should stay independently
//                          auditable from a finding's message.
//
// A path found under ANY of these roots is present — no finding is emitted.
// When a path is absent under every root, the finding's `message` names every
// root that was searched (label + base path), so a future false positive is
// diagnosable from the carryover entry alone, without re-reading the source
// (see [`ResolutionRoot`] and [`resolve_referenced_path`]).

/// One named search location [`resolve_referenced_path`] tries, in the
/// order it appears in the list passed to it. See the module-level scope
/// contract's "Resolution is fleet-wide, not repo-local" section for the
/// concrete order this crate builds.
#[derive(Debug, Clone)]
pub struct ResolutionRoot {
    /// Human-readable label for this root — e.g. `repo:mev`, `brain-root`,
    /// `base-template`, `owner:base-template`. Surfaced verbatim in a
    /// finding's `message` so a future false positive is diagnosable from
    /// the carryover entry alone.
    pub label: String,
    /// Absolute base path a referenced path is joined onto.
    pub base: PathBuf,
}

impl ResolutionRoot {
    /// Construct a root from a label and base path.
    #[must_use]
    pub fn new(label: impl Into<String>, base: PathBuf) -> Self {
        ResolutionRoot {
            label: label.into(),
            base,
        }
    }
}

/// Resolve `raw` (a raw, un-normalized reference as extracted by
/// [`extract_referenced_script_paths`]) against `roots`, in order, and
/// return the FIRST root under which `root.base.join(raw)` exists — `None`
/// only when it exists under none of them.
///
/// `roots` order encodes precedence, not preference among ties: since this
/// only answers "does it exist anywhere", which root is returned when more
/// than one would match does not change the presence/absence verdict a
/// finding is based on — it only changes which root's label a caller
/// wanting a single "found here" answer would report. The message built by
/// [`referenced_path_absent_findings`] instead always lists every root that
/// was searched, not just the one (if any) that matched, which is why order
/// among matching roots is otherwise unobservable from a finding.
#[must_use]
pub fn resolve_referenced_path<'a>(
    raw: &str,
    roots: &'a [ResolutionRoot],
) -> Option<&'a ResolutionRoot> {
    roots.iter().find(|root| root.base.join(raw).exists())
}

/// One file whose contents are scanned for referenced script/generator
/// paths, together with the repo it belongs to, that repo's root (root (a)
/// in the module-level resolution order — the original, only check before
/// MV.ticket.graph-findings-path-resolution), and the additional roots (b),
/// (c), (d) that apply to this particular source.
#[derive(Debug, Clone)]
pub struct ReferencingSource {
    /// Owning repo slug (from `brain.toml`'s `[[repos]]`).
    pub repo: String,
    /// Absolute path to the repo's own root directory — relative
    /// references extracted from `contents` resolve against this FIRST
    /// (root (a)), not the fleet HQ root.
    pub repo_root: PathBuf,
    /// The command or spec file the reference was found in (for
    /// diagnostics/messages only — never fed into `finding_id`).
    pub file_path: PathBuf,
    /// Raw file contents to scan.
    pub contents: String,
    /// Additional resolution roots beyond `repo_root` — the brain root,
    /// base-template, and (for a synced command) its owning repo, in the
    /// module-level resolution order (b), (c), (d). Empty is valid (e.g. in
    /// tests exercising only the repo-local case) — [`referenced_path_absent_findings`]
    /// always prepends `repo_root` as root (a) regardless of this field.
    pub resolution_roots: Vec<ResolutionRoot>,
}

/// Whether `ext`-less-dot suffix counts as a "script or generator" for this
/// detector. See the module-level scope contract above for why only these
/// two.
fn is_script_extension(token: &str) -> bool {
    token.ends_with(".py") || token.ends_with(".sh")
}

/// Extract candidate script/generator path references from raw file
/// contents.
///
/// Pure text scan, independent of file format (markdown command or JSON
/// spec) — both are scanned as flat text, since the reference shape (a
/// path-like token ending `.py`/`.sh`) shows up the same way whether it
/// sits inside a backtick span, a JSON string value, or a shell
/// invocation. See the module-level scope contract for the exact character
/// class and the `/`-required rule that excludes bare filenames and URLs.
#[must_use]
pub fn extract_referenced_script_paths(contents: &str) -> Vec<String> {
    let is_path_char = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/');

    let mut tokens: Vec<&str> = Vec::new();
    let mut start: Option<usize> = None;
    for (i, c) in contents.char_indices() {
        if is_path_char(c) {
            if start.is_none() {
                start = Some(i);
            }
        } else if let Some(s) = start.take() {
            tokens.push(&contents[s..i]);
        }
    }
    if let Some(s) = start {
        tokens.push(&contents[s..]);
    }

    let mut found = Vec::new();
    for raw in tokens {
        // Sentence-terminal punctuation ('.') is itself a path character, so a
        // reference immediately followed by a period with no space (end of a
        // sentence) would otherwise swallow it into the token. Trim trailing
        // dots before checking the extension.
        let trimmed = raw.trim_end_matches('.');
        if trimmed.contains('/') && is_script_extension(trimmed) {
            found.push(trimmed.to_string());
        }
    }
    found
}

/// Detector 2 — `referenced-path-absent`, over already-extracted sources.
/// Pure with respect to disk beyond the existence check itself: callers do
/// the file discovery ([`detect_referenced_path_absent`] is the disk-facing
/// wrapper), which keeps this half unit-testable without a fixture
/// directory per case.
///
/// For every candidate reference [`extract_referenced_script_paths`] finds
/// in each `source`, resolves it against that source's `repo_root` (never
/// the fleet HQ root — a repo's command references its own scripts
/// relatively) and reports a finding when it does not exist on disk.
/// `subject` is [`normalize_referenced_path`] applied to the raw reference,
/// so the *same* missing path referenced from different repos — or
/// spelled with a different leading prefix — produces rows sharing one
/// `finding_id`, per the block record's `render-spec.py` case.
///
/// Resolution is fleet-wide (MV.ticket.graph-findings-path-resolution task
/// 1): each source's `repo_root` is always tried first (root (a)),
/// followed by `source.resolution_roots` in the order the caller built them
/// (roots (b)/(c)/(d) — see the module-level scope contract). A path found
/// under ANY root is present; a finding is emitted only when it resolves
/// under none of them, and its `message` names every root that was
/// searched (label + base path) so a future false positive is diagnosable
/// from the carryover entry alone.
#[must_use]
pub fn referenced_path_absent_findings(sources: &[ReferencingSource]) -> Vec<GraphFinding> {
    let mut findings = Vec::new();
    for source in sources {
        let mut roots: Vec<ResolutionRoot> = Vec::with_capacity(1 + source.resolution_roots.len());
        roots.push(ResolutionRoot::new(
            format!("repo:{}", source.repo),
            source.repo_root.clone(),
        ));
        roots.extend(source.resolution_roots.iter().cloned());

        for raw_path in extract_referenced_script_paths(&source.contents) {
            if resolve_referenced_path(&raw_path, &roots).is_some() {
                continue;
            }
            let subject = normalize_referenced_path(&raw_path);
            let searched = roots
                .iter()
                .map(|root| format!("{} ({})", root.label, root.base.display()))
                .collect::<Vec<_>>()
                .join(", ");
            let message = format!(
                "{} references '{}', which does not exist under any searched root: {} (repo '{}')",
                source.file_path.display(),
                raw_path,
                searched,
                source.repo,
            );
            // Brain-root-relative, per `GraphFinding::clears_when`'s
            // reconciliation note: `raw_path` (never the normalized
            // `subject`, which is lossy) lets `mev carryover`'s evaluator
            // reproduce roots (a)/(b) of the search above — brain_root.join
            // (raw_path) is root (b) exactly, and the evaluator's owning-
            // repo fallback, repo_paths[repo].join(raw_path), is root (a)
            // exactly. Roots (c)/(d) (`base-template`, a synced command's
            // owner) are outside the evaluator's two-root reach, but this
            // finding was only emitted because the path resolved under NONE
            // of the four roots, so there is no (c)/(d)-only verdict this
            // predicate needs to reproduce.
            let clears_when = ClearsWhenPredicate::FileExists {
                path: raw_path.clone(),
                note: Some(format!(
                    "clears when '{raw_path}' resolves under the referencing repo's \
                     root or the brain root"
                )),
            };
            findings.push(GraphFinding {
                detector: DetectorClass::ReferencedPathAbsent,
                repo: source.repo.clone(),
                subject: subject.clone(),
                message,
                finding_id: finding_id(DetectorClass::ReferencedPathAbsent, &subject),
                clears_when: Some(clears_when),
            });
        }
    }
    findings
}

/// Disk-facing wrapper for [`referenced_path_absent_findings`]: for every
/// `[[repos]]` entry in `config`, walks `<repo>/.claude/commands/` (`.md`)
/// and `<repo>/planning/blocks/` (`.json`) under `root`, reads each file,
/// and reduces the collected [`ReferencingSource`]s through the pure
/// detector above.
///
/// `.follow_links(true)` on both walks — see the module-level scope
/// contract's symlink-trap note: a repo's `planning/` is itself a symlink
/// into the `_planning/` vault, and the walk must descend into it to find
/// `planning/blocks/*.json` at all.
///
/// Builds each source's fleet-wide resolution roots (b)/(c)/(d) — see the
/// module-level scope contract — from `config`: root (b) is `root` itself
/// (the brain/HQ root); root (c) is the `base-template` `[[repos]]` entry's
/// own root, added unconditionally for every source, present or absent, so
/// its presence never depends on file type; root (d) is added ONLY when the
/// source is itself a `.claude/commands/*.md` file whose basename also
/// exists under `base-template/.claude/commands/` (the "synced command"
/// test the block record names) — resolved through the same `base-template`
/// `[[repos]]` entry rather than a second hardcoded path.
#[must_use]
pub fn detect_referenced_path_absent(
    root: &Path,
    config: &BrainConfig,
) -> (Vec<GraphFinding>, Vec<Diagnostic>) {
    let mut sources = Vec::new();
    let mut diags = Vec::new();

    let resolve_repo_root = |repo_path: &str| -> PathBuf {
        let trimmed = repo_path.trim();
        if trimmed.is_empty() || trimmed == "." {
            root.to_path_buf()
        } else {
            root.join(trimmed)
        }
    };

    // Root (c)/(d)'s shared base: base-template's OWN root, resolved
    // through its `[[repos]]` entry (not a hardcoded "base-template"
    // literal) so a future repo_path change to that entry is picked up
    // automatically. `None` when the corpus has no `base-template` entry
    // (e.g. a fixture config in a test) — roots (c)/(d) are then simply
    // never added, which degrades to the pre-ticket repo-local-only
    // behavior rather than panicking.
    let base_template_root: Option<PathBuf> = config
        .repos
        .iter()
        .find(|r| r.slug == "base-template")
        .map(|r| resolve_repo_root(&r.repo_path));
    let base_template_commands_dir = base_template_root
        .as_ref()
        .map(|r| r.join(".claude").join("commands"));

    for repo in &config.repos {
        let repo_root = resolve_repo_root(&repo.repo_path);

        for (scan_dir, ext, is_command_dir) in [
            (repo_root.join(".claude").join("commands"), "md", true),
            (repo_root.join("planning").join("blocks"), "json", false),
        ] {
            if !scan_dir.exists() {
                continue;
            }
            let iter = walkdir::WalkDir::new(&scan_dir)
                .follow_links(true)
                .into_iter();
            for entry in iter {
                let entry = match entry {
                    Ok(e) => e,
                    Err(e) => {
                        diags.push(Diagnostic::error(&scan_dir, "", format!("walk error: {e}")));
                        continue;
                    }
                };
                if !entry.file_type().is_file() {
                    continue;
                }
                if entry.path().extension().and_then(|e| e.to_str()) != Some(ext) {
                    continue;
                }
                match std::fs::read_to_string(entry.path()) {
                    Ok(contents) => {
                        let mut resolution_roots =
                            vec![ResolutionRoot::new("brain-root", root.to_path_buf())];
                        if let Some(bt_root) = &base_template_root {
                            resolution_roots
                                .push(ResolutionRoot::new("base-template", bt_root.clone()));

                            let is_synced_command = is_command_dir
                                && base_template_commands_dir
                                    .as_ref()
                                    .and_then(|dir| {
                                        entry.path().file_name().map(|name| dir.join(name))
                                    })
                                    .is_some_and(|candidate| candidate.exists());
                            if is_synced_command {
                                resolution_roots.push(ResolutionRoot::new(
                                    "owner:base-template",
                                    bt_root.clone(),
                                ));
                            }
                        }

                        sources.push(ReferencingSource {
                            repo: repo.slug.clone(),
                            repo_root: repo_root.clone(),
                            file_path: entry.path().to_path_buf(),
                            contents,
                            resolution_roots,
                        });
                    }
                    Err(e) => {
                        diags.push(Diagnostic::error(
                            entry.path(),
                            "",
                            format!("could not read file: {e}"),
                        ));
                    }
                }
            }
        }
    }

    let findings = referenced_path_absent_findings(&sources);
    (findings, diags)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_class_same_subject_yields_same_id() {
        let a = finding_id(DetectorClass::UnregisteredLaneBlock, "mev:MV.1.A");
        let b = finding_id(DetectorClass::UnregisteredLaneBlock, "mev:MV.1.A");
        assert_eq!(a, b);
    }

    #[test]
    fn different_class_same_subject_yields_different_id() {
        let a = finding_id(
            DetectorClass::UnregisteredLaneBlock,
            "scripts/render_spec.py",
        );
        let b = finding_id(
            DetectorClass::ReferencedPathAbsent,
            "scripts/render_spec.py",
        );
        assert_ne!(a, b);
    }

    #[test]
    fn different_subject_same_class_yields_different_id() {
        let a = finding_id(
            DetectorClass::ReferencedPathAbsent,
            "scripts/render_spec.py",
        );
        let b = finding_id(DetectorClass::ReferencedPathAbsent, "scripts/other.py");
        assert_ne!(a, b);
    }

    #[test]
    fn finding_id_excludes_repo_and_is_stable_across_calls() {
        // The block record requires finding_id to be derived ONLY from
        // (detector class, normalized subject) -- never the owning repo, a
        // path it was found in, a timestamp, or an index. Simulate three
        // "repos" independently computing the id for the identical
        // normalized subject and assert they all agree, and that repeated
        // calls (standing in for repeated runs) are identical too.
        let subject = "scripts/render_spec.py";
        let from_mev = finding_id(DetectorClass::ReferencedPathAbsent, subject);
        let from_base_template = finding_id(DetectorClass::ReferencedPathAbsent, subject);
        let from_engine_rs = finding_id(DetectorClass::ReferencedPathAbsent, subject);
        assert_eq!(from_mev, from_base_template);
        assert_eq!(from_base_template, from_engine_rs);

        // Re-running "later" (nothing time-dependent in the function) still
        // agrees.
        let again = finding_id(DetectorClass::ReferencedPathAbsent, subject);
        assert_eq!(from_mev, again);
    }

    #[test]
    fn three_path_spellings_normalize_to_one_subject() {
        let bare = normalize_referenced_path("scripts/render_spec.py");
        let dot_relative = normalize_referenced_path("./scripts/render_spec.py");
        let repo_prefixed = normalize_referenced_path("base-template/scripts/render_spec.py");

        assert_eq!(bare, "scripts/render_spec.py");
        assert_eq!(dot_relative, bare);
        assert_eq!(repo_prefixed, bare);

        // And therefore they hash to one finding_id.
        let id_bare = finding_id(DetectorClass::ReferencedPathAbsent, &bare);
        let id_dot = finding_id(DetectorClass::ReferencedPathAbsent, &dot_relative);
        let id_prefixed = finding_id(DetectorClass::ReferencedPathAbsent, &repo_prefixed);
        assert_eq!(id_bare, id_dot);
        assert_eq!(id_dot, id_prefixed);
    }

    #[test]
    fn normalize_preserves_distinct_parent_directories() {
        // Two scripts that merely share a filename in different parent
        // directories must NOT collapse to the same subject.
        let a = normalize_referenced_path("scripts/build.sh");
        let b = normalize_referenced_path("hooks/build.sh");
        assert_ne!(a, b);
    }

    #[test]
    fn normalize_single_component_path_is_unchanged() {
        assert_eq!(normalize_referenced_path("README.md"), "README.md");
    }

    #[test]
    fn detector_class_tag_is_kebab_case_and_stable() {
        assert_eq!(
            DetectorClass::UnregisteredLaneBlock.tag(),
            "unregistered-lane-block"
        );
        assert_eq!(
            DetectorClass::ReferencedPathAbsent.tag(),
            "referenced-path-absent"
        );
    }

    #[test]
    fn report_from_findings_derives_counts_from_rows() {
        let findings = vec![
            GraphFinding {
                detector: DetectorClass::UnregisteredLaneBlock,
                repo: "mev".to_string(),
                subject: "mev:MV.1.A".to_string(),
                message: "unregistered".to_string(),
                finding_id: finding_id(DetectorClass::UnregisteredLaneBlock, "mev:MV.1.A"),
                clears_when: None,
            },
            GraphFinding {
                detector: DetectorClass::ReferencedPathAbsent,
                repo: "mev".to_string(),
                subject: "scripts/render_spec.py".to_string(),
                message: "missing".to_string(),
                finding_id: finding_id(
                    DetectorClass::ReferencedPathAbsent,
                    "scripts/render_spec.py",
                ),
                clears_when: None,
            },
            GraphFinding {
                detector: DetectorClass::ReferencedPathAbsent,
                repo: "base-template".to_string(),
                subject: "scripts/render_spec.py".to_string(),
                message: "missing".to_string(),
                finding_id: finding_id(
                    DetectorClass::ReferencedPathAbsent,
                    "scripts/render_spec.py",
                ),
                clears_when: None,
            },
        ];

        let report = GraphFindingsReport::from_findings(findings);
        assert_eq!(report.total, 3);
        assert_eq!(report.unregistered_lane_block, 1);
        assert_eq!(report.referenced_path_absent, 2);
        assert_eq!(report.findings.len(), 3);
    }

    #[test]
    fn report_default_is_empty() {
        let report = GraphFindingsReport::default();
        assert_eq!(report.total, 0);
        assert_eq!(report.unregistered_lane_block, 0);
        assert_eq!(report.referenced_path_absent, 0);
        assert!(report.findings.is_empty());
    }

    // -----------------------------------------------------------------
    // Detector 1: unregistered-lane-block
    // -----------------------------------------------------------------

    use crate::brain::lane_segments::LaneBlockRef;
    use crate::brain::state::{Focus, Track, TrackBlock};
    use std::path::PathBuf;

    fn lane_block_ref(id: &str, repo: &str) -> LaneBlockRef {
        LaneBlockRef {
            id: id.to_string(),
            line: 1,
            origin_roadmap: Some("alpha".to_string()),
            repo: repo.to_string(),
        }
    }

    fn lane_file(lane: &str, roadmap: &str, blocks: Vec<LaneBlockRef>) -> LaneFile {
        LaneFile {
            roadmap: roadmap.to_string(),
            lane: lane.to_string(),
            path: PathBuf::from(format!("planning/roadmaps/{roadmap}/lane-{lane}.json")),
            blocks,
            directives: None,
        }
    }

    /// Minimal `BrainConfig` fixture: one `[[repos]]` entry per given slug,
    /// with `repo_path` set identical to the slug (matching how
    /// [`state_source`]'s fake `abs_path`es are built, so a computed
    /// `<repo_path>/planning/state.json` predicate path lines up with what
    /// these tests otherwise assert).
    fn test_config(repos: &[&str]) -> BrainConfig {
        BrainConfig {
            repos: repos
                .iter()
                .map(|slug| crate::brain::config::RepoEntry {
                    slug: (*slug).to_string(),
                    repo_path: (*slug).to_string(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
    }

    fn state_source(repo: &str) -> StateSource {
        StateSource {
            repo_slug: repo.to_string(),
            abs_path: PathBuf::from(format!("/{repo}/planning/state.json")),
            expected_kind: "project",
        }
    }

    fn track_block(id: &str) -> TrackBlock {
        TrackBlock {
            id: id.to_string(),
            title: format!("Block {id}"),
            status: None,
            depends_on: Vec::new(),
            ..Default::default()
        }
    }

    fn project_file(repo: &str, blocks: Vec<TrackBlock>) -> StateFile {
        StateFile {
            repo: repo.to_string(),
            kind: "project".to_string(),
            updated: "2026-01-01".to_string(),
            focus: Focus::default(),
            tracks: vec![Track {
                title: "Phase 1".to_string(),
                blocks,
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn registered_lane_block_produces_no_finding() {
        let lane = lane_file("substrate", "alpha", vec![lane_block_ref("MV.1.A", "mev")]);
        let state_files = vec![(
            state_source("mev"),
            project_file("mev", vec![track_block("MV.1.A")]),
        )];

        let config = test_config(&["mev"]);
        let (findings, diags) =
            unregistered_lane_block_findings(&[lane], &[], &state_files, &config);
        assert!(
            findings.is_empty(),
            "expected no findings, got {findings:?}"
        );
        assert!(diags.is_empty());
    }

    #[test]
    fn unregistered_lane_block_produces_exactly_one_finding() {
        let lane = lane_file("substrate", "alpha", vec![lane_block_ref("MV.1.A", "mev")]);
        // The registered id is a DIFFERENT id in the same repo -- MV.1.A itself is
        // never registered.
        let state_files = vec![(
            state_source("mev"),
            project_file("mev", vec![track_block("MV.9.Z")]),
        )];

        let config = test_config(&["mev"]);
        let (findings, diags) =
            unregistered_lane_block_findings(&[lane], &[], &state_files, &config);
        assert_eq!(
            findings.len(),
            1,
            "expected exactly one finding, got {findings:?}"
        );
        assert!(diags.is_empty());

        let f = &findings[0];
        assert_eq!(f.detector, DetectorClass::UnregisteredLaneBlock);
        assert_eq!(f.repo, "mev");
        assert_eq!(f.subject, "mev:MV.1.A");
        assert_eq!(
            f.finding_id,
            finding_id(DetectorClass::UnregisteredLaneBlock, "mev:MV.1.A")
        );
    }

    #[test]
    fn id_registered_in_a_different_repo_still_produces_a_finding() {
        // The lane entry names repo "mev", but the ONLY state.json registering
        // "MV.1.A" belongs to "base-template". Ownership resolves against the
        // lane entry's own authored `repo`, never against wherever the id
        // happens to be registered -- so this must still fire.
        let lane = lane_file("substrate", "alpha", vec![lane_block_ref("MV.1.A", "mev")]);
        let state_files = vec![(
            state_source("base-template"),
            project_file("base-template", vec![track_block("MV.1.A")]),
        )];

        let config = test_config(&["mev", "base-template"]);
        let (findings, _diags) =
            unregistered_lane_block_findings(&[lane], &[], &state_files, &config);
        assert_eq!(
            findings.len(),
            1,
            "expected exactly one finding, got {findings:?}"
        );
        assert_eq!(findings[0].repo, "mev");
        assert_eq!(findings[0].subject, "mev:MV.1.A");
    }

    #[test]
    fn unparseable_lane_record_surfaces_an_error_not_a_clean_empty_result() {
        // Reuse the existing malformed-record fixture (lane_segments.rs's own
        // "unknown top-level key" regression) rather than duplicating one: run
        // real discovery over it, then feed the diagnostics it returns through
        // the detector and assert they survive -- this is standing rule 11's
        // positive control applied to this detector specifically. A detector
        // that silently swallowed discover_lane_files' diagnostics would report
        // zero findings here for exactly the wrong reason (it could not read
        // its input, not because the corpus is clean).
        let dir = crate::testsupport::unique_temp_dir("mev-graph-findings-malformed-lane");
        let fixture = std::fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/lane_json/unknown_key_lane.json"),
        )
        .expect("fixture must exist");
        let target = dir.join("planning/roadmaps/alpha/lane-docs-sync.json");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, fixture).unwrap();

        let (lane_files, lane_diags) = discover_lane_files(&dir);
        assert!(
            lane_files.is_empty(),
            "the malformed record must not be parsed into a usable LaneFile"
        );
        assert!(
            !lane_diags.is_empty(),
            "discover_lane_files must surface a diagnostic for the malformed record"
        );

        let config = test_config(&[]);
        let (findings, diags) =
            unregistered_lane_block_findings(&lane_files, &lane_diags, &[], &config);
        assert!(
            findings.is_empty(),
            "no lane blocks were discoverable, so there is nothing to report as unregistered"
        );
        assert!(
            diags.iter().any(|d| d.severity == crate::Severity::Error),
            "the parse failure must surface as an error diagnostic, not silently \
             produce a clean-looking empty result: {diags:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------
    // Detector 2: referenced-path-absent
    // -----------------------------------------------------------------

    #[test]
    fn extract_finds_slash_qualified_script_reference() {
        let contents = "Run `scripts/render_spec.py --check` before committing.";
        let found = extract_referenced_script_paths(contents);
        assert_eq!(found, vec!["scripts/render_spec.py".to_string()]);
    }

    #[test]
    fn extract_finds_sh_reference_in_json_style_text() {
        let contents = r#"{"validation_commands": ["scripts/check_things.sh --all"]}"#;
        let found = extract_referenced_script_paths(contents);
        assert_eq!(found, vec!["scripts/check_things.sh".to_string()]);
    }

    #[test]
    fn extract_trims_trailing_sentence_period() {
        let contents = "See scripts/render_spec.py.";
        let found = extract_referenced_script_paths(contents);
        assert_eq!(found, vec!["scripts/render_spec.py".to_string()]);
    }

    #[test]
    fn extract_ignores_bare_filename_with_no_directory() {
        // No '/' — deliberately excluded per the scope contract (too easy
        // to collide with an unrelated word).
        let contents = "run render_spec.py directly";
        assert!(extract_referenced_script_paths(contents).is_empty());
    }

    #[test]
    fn extract_ignores_non_script_extensions() {
        let contents = "see src/brain/graph_findings.rs and docs/cli.md";
        assert!(extract_referenced_script_paths(contents).is_empty());
    }

    #[test]
    fn extract_ignores_bare_domain_url() {
        // '://' is not a path character, so a bare domain reference with no
        // script-shaped path segment never reaches the extension check.
        let found = extract_referenced_script_paths("see https://example.com for docs");
        assert!(found.is_empty());
    }

    #[test]
    fn extract_multiple_references_in_one_file() {
        let contents = "scripts/a.py then scripts/b.sh then done";
        let found = extract_referenced_script_paths(contents);
        assert_eq!(
            found,
            vec!["scripts/a.py".to_string(), "scripts/b.sh".to_string()]
        );
    }

    fn referencing_source(repo: &str, repo_root: &Path, contents: &str) -> ReferencingSource {
        ReferencingSource {
            repo: repo.to_string(),
            repo_root: repo_root.to_path_buf(),
            file_path: repo_root.join(".claude/commands/example.md"),
            contents: contents.to_string(),
            resolution_roots: Vec::new(),
        }
    }

    #[test]
    fn existing_referenced_path_produces_no_finding() {
        let dir = crate::testsupport::unique_temp_dir("mev-graph-findings-existing-ref");
        std::fs::create_dir_all(dir.join("scripts")).unwrap();
        std::fs::write(dir.join("scripts/render_spec.py"), b"# exists").unwrap();

        let source = referencing_source(
            "mev",
            &dir,
            "invoke `scripts/render_spec.py` to render the spec",
        );
        let findings = referenced_path_absent_findings(&[source]);
        assert!(
            findings.is_empty(),
            "expected no findings for an existing path, got {findings:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_referenced_path_produces_a_finding() {
        let dir = crate::testsupport::unique_temp_dir("mev-graph-findings-missing-ref");
        std::fs::create_dir_all(&dir).unwrap();

        let source = referencing_source(
            "mev",
            &dir,
            "invoke `scripts/render_spec.py` to render the spec",
        );
        let findings = referenced_path_absent_findings(&[source]);
        assert_eq!(
            findings.len(),
            1,
            "expected exactly one finding, got {findings:?}"
        );
        let f = &findings[0];
        assert_eq!(f.detector, DetectorClass::ReferencedPathAbsent);
        assert_eq!(f.repo, "mev");
        assert_eq!(f.subject, "scripts/render_spec.py");
        assert_eq!(
            f.finding_id,
            finding_id(
                DetectorClass::ReferencedPathAbsent,
                "scripts/render_spec.py"
            )
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn referenced_path_reachable_only_through_a_planning_symlink_produces_nothing() {
        // The symlink trap: `planning/` in the real corpus is a symlink into
        // a `_planning/` vault. Build the same shape here -- a real vault
        // directory holding the referenced file, and `planning` as a
        // symlink to it -- and assert std::path resolution (used directly
        // by the detector, no special-casing) already follows it.
        let dir = crate::testsupport::unique_temp_dir("mev-graph-findings-symlink-ref");
        let vault = dir.join("_planning_vault");
        std::fs::create_dir_all(vault.join("scripts")).unwrap();
        std::fs::write(vault.join("scripts/render_spec.py"), b"# exists via vault").unwrap();

        #[cfg(unix)]
        std::os::unix::fs::symlink(&vault, dir.join("planning")).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(&vault, dir.join("planning")).unwrap();

        let source = referencing_source(
            "mev",
            &dir,
            "invoke `planning/scripts/render_spec.py` to render the spec",
        );
        let findings = referenced_path_absent_findings(&[source]);
        assert!(
            findings.is_empty(),
            "a path reachable only through a planning/ symlink must not be \
             reported absent, got {findings:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn same_missing_path_in_two_repos_produces_two_rows_sharing_one_finding_id() {
        let dir_a = crate::testsupport::unique_temp_dir("mev-graph-findings-two-repo-a");
        let dir_b = crate::testsupport::unique_temp_dir("mev-graph-findings-two-repo-b");
        std::fs::create_dir_all(&dir_a).unwrap();
        std::fs::create_dir_all(&dir_b).unwrap();

        let source_a = referencing_source(
            "base-template",
            &dir_a,
            "invoke `scripts/render_spec.py` here",
        );
        // A different spelling of the same reference, per the block record's
        // three-repo render-spec.py case -- normalization must still collapse
        // it to the same subject/finding_id.
        let source_b = referencing_source("mev", &dir_b, "invoke `./scripts/render_spec.py` here");

        let findings = referenced_path_absent_findings(&[source_a, source_b]);
        assert_eq!(
            findings.len(),
            2,
            "expected one row per repo, got {findings:?}"
        );
        assert_eq!(findings[0].finding_id, findings[1].finding_id);
        assert_ne!(findings[0].repo, findings[1].repo);

        let _ = std::fs::remove_dir_all(&dir_a);
        let _ = std::fs::remove_dir_all(&dir_b);
    }

    // -----------------------------------------------------------------
    // Task 2 -- resolver test suite (MV.ticket.graph-findings-path-resolution)
    // -----------------------------------------------------------------

    /// The load-bearing positive control the original block LACKED: a
    /// script that lives ONLY under `base-template/scripts/`, referenced
    /// from a DIFFERENT repo's synced command, must produce zero findings.
    /// This is the exact shape of the 81% false-positive class measured
    /// live 2026-08-23 -- a real file reported "absent" in up to 19 repos
    /// because the pre-fix detector only ever checked the referencing
    /// repo's own root.
    #[test]
    fn path_present_only_in_base_template_referenced_from_another_repo_produces_zero_findings() {
        let repo_dir = crate::testsupport::unique_temp_dir("mev-graph-findings-fleet-repo");
        let bt_dir = crate::testsupport::unique_temp_dir("mev-graph-findings-fleet-bt");
        std::fs::create_dir_all(&repo_dir).unwrap();
        std::fs::create_dir_all(bt_dir.join("scripts")).unwrap();
        std::fs::write(
            bt_dir.join("scripts/fleet_concurrency_check.py"),
            b"# lives only in base-template",
        )
        .unwrap();

        let source = ReferencingSource {
            repo: "engine-rs".to_string(),
            repo_root: repo_dir.clone(),
            file_path: repo_dir.join(".claude/commands/orchestrate.md"),
            contents: "run `scripts/fleet_concurrency_check.py` before merging".to_string(),
            resolution_roots: vec![
                ResolutionRoot::new("brain-root", repo_dir.join("does-not-exist-brain-root")),
                ResolutionRoot::new("base-template", bt_dir.clone()),
                ResolutionRoot::new("owner:base-template", bt_dir.clone()),
            ],
        };

        let findings = referenced_path_absent_findings(&[source]);
        assert!(
            findings.is_empty(),
            "a script that exists only in base-template/scripts/, referenced \
             from another repo's synced command, must not be reported absent \
             -- this is the whole 81%, got {findings:?}"
        );

        let _ = std::fs::remove_dir_all(&repo_dir);
        let _ = std::fs::remove_dir_all(&bt_dir);
    }

    /// The fix must not simply silence the detector: a path absent under
    /// every searched root still produces exactly one finding per
    /// referencing repo.
    #[test]
    fn path_absent_under_every_root_still_reports_one_finding_per_repo() {
        let dir_a = crate::testsupport::unique_temp_dir("mev-graph-findings-absent-everywhere-a");
        let dir_b = crate::testsupport::unique_temp_dir("mev-graph-findings-absent-everywhere-b");
        let brain_dir = crate::testsupport::unique_temp_dir("mev-graph-findings-absent-brain");
        let bt_dir = crate::testsupport::unique_temp_dir("mev-graph-findings-absent-bt");
        std::fs::create_dir_all(&dir_a).unwrap();
        std::fs::create_dir_all(&dir_b).unwrap();
        std::fs::create_dir_all(&brain_dir).unwrap();
        std::fs::create_dir_all(&bt_dir).unwrap();

        let roots = vec![
            ResolutionRoot::new("brain-root", brain_dir.clone()),
            ResolutionRoot::new("base-template", bt_dir.clone()),
        ];

        let source_a = ReferencingSource {
            repo: "engine-rs".to_string(),
            repo_root: dir_a.clone(),
            file_path: dir_a.join(".claude/commands/example.md"),
            contents: "invoke `scripts/nowhere.py` please".to_string(),
            resolution_roots: roots.clone(),
        };
        let source_b = ReferencingSource {
            repo: "mev".to_string(),
            repo_root: dir_b.clone(),
            file_path: dir_b.join(".claude/commands/example.md"),
            contents: "invoke `scripts/nowhere.py` please".to_string(),
            resolution_roots: roots,
        };

        let findings = referenced_path_absent_findings(&[source_a, source_b]);
        assert_eq!(
            findings.len(),
            2,
            "a fleet-wide-absent path must still report once per referencing \
             repo, not be silenced -- got {findings:?}"
        );

        let _ = std::fs::remove_dir_all(&dir_a);
        let _ = std::fs::remove_dir_all(&dir_b);
        let _ = std::fs::remove_dir_all(&brain_dir);
        let _ = std::fs::remove_dir_all(&bt_dir);
    }

    /// Regression on today's only working case: a path present only in the
    /// referencing repo's own root still resolves clean once fleet-wide
    /// roots are added alongside it.
    #[test]
    fn path_present_only_in_referencing_repo_still_resolves_clean() {
        let dir = crate::testsupport::unique_temp_dir("mev-graph-findings-repo-local-only");
        let brain_dir = crate::testsupport::unique_temp_dir("mev-graph-findings-repo-local-brain");
        std::fs::create_dir_all(dir.join("scripts")).unwrap();
        std::fs::write(dir.join("scripts/local_only.py"), b"# repo-local").unwrap();
        std::fs::create_dir_all(&brain_dir).unwrap();

        let source = ReferencingSource {
            repo: "mev".to_string(),
            repo_root: dir.clone(),
            file_path: dir.join(".claude/commands/example.md"),
            contents: "invoke `scripts/local_only.py` here".to_string(),
            resolution_roots: vec![ResolutionRoot::new("brain-root", brain_dir.clone())],
        };

        let findings = referenced_path_absent_findings(&[source]);
        assert!(
            findings.is_empty(),
            "a path present only in the referencing repo must still resolve \
             clean, got {findings:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&brain_dir);
    }

    /// A path resolvable only through the brain root (not the referencing
    /// repo, not base-template) still resolves clean.
    #[test]
    fn path_resolvable_only_through_brain_root_resolves_clean() {
        let dir = crate::testsupport::unique_temp_dir("mev-graph-findings-brain-root-only");
        let brain_dir = crate::testsupport::unique_temp_dir("mev-graph-findings-brain-root-only-b");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(brain_dir.join("scripts")).unwrap();
        std::fs::write(brain_dir.join("scripts/hq_only.py"), b"# brain-root only").unwrap();

        let source = ReferencingSource {
            repo: "mev".to_string(),
            repo_root: dir.clone(),
            file_path: dir.join(".claude/commands/example.md"),
            contents: "invoke `scripts/hq_only.py` here".to_string(),
            resolution_roots: vec![ResolutionRoot::new("brain-root", brain_dir.clone())],
        };

        let findings = referenced_path_absent_findings(&[source]);
        assert!(
            findings.is_empty(),
            "a path resolvable only through the brain root must resolve \
             clean, got {findings:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&brain_dir);
    }

    /// The recorded search order must appear in the emitted finding's
    /// `message` -- by label and base path -- so a future false positive is
    /// diagnosable from the carryover entry alone, without re-reading the
    /// source.
    #[test]
    fn recorded_search_order_appears_in_finding_message() {
        let dir = crate::testsupport::unique_temp_dir("mev-graph-findings-search-order");
        let brain_dir = crate::testsupport::unique_temp_dir("mev-graph-findings-search-order-b");
        let bt_dir = crate::testsupport::unique_temp_dir("mev-graph-findings-search-order-c");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(&brain_dir).unwrap();
        std::fs::create_dir_all(&bt_dir).unwrap();

        let source = ReferencingSource {
            repo: "mev".to_string(),
            repo_root: dir.clone(),
            file_path: dir.join(".claude/commands/example.md"),
            contents: "invoke `scripts/nowhere.py` here".to_string(),
            resolution_roots: vec![
                ResolutionRoot::new("brain-root", brain_dir.clone()),
                ResolutionRoot::new("base-template", bt_dir.clone()),
            ],
        };

        let findings = referenced_path_absent_findings(&[source]);
        assert_eq!(findings.len(), 1, "expected one finding, got {findings:?}");
        let message = &findings[0].message;
        assert!(
            message.contains(&format!("repo:mev ({})", dir.display())),
            "message must name the repo-local root, got: {message}"
        );
        assert!(
            message.contains(&format!("brain-root ({})", brain_dir.display())),
            "message must name the brain-root, got: {message}"
        );
        assert!(
            message.contains(&format!("base-template ({})", bt_dir.display())),
            "message must name base-template, got: {message}"
        );

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&brain_dir);
        let _ = std::fs::remove_dir_all(&bt_dir);
    }
}
