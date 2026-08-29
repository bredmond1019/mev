//! Lane record discovery and parsing — `MV.13.A` Task 1, replaced for `lane.json` by
//! `MV.17.A` Task 2.
//!
//! A "lane record" (`lane-<name>.json`) is the authored execution order for one
//! roadmap's slice of work: an ordered `blocks[]` array, each entry naming a block ID
//! plus the repo it runs in and the roadmap that originally allocated it
//! (`origin_roadmap`). It replaces the earlier line-oriented `.txt` lane-file format and
//! its comment-directive grammar (`# ORIGIN:`, `# HELD-UNTIL:`, `# BUDGET:`,
//! `# EXCLUSIVE-REPOS:`) entirely — there is no comment syntax to mis-read as a
//! directive, and because every block carries its own authored `repo` and
//! `origin_roadmap`, there is no unannotated double-claim to resolve. Both defect
//! classes this module used to detect and diagnose (`E_LANE_DIRECTIVE_UNRECOGNISED`/
//! `E_LANE_DIRECTIVE_MALFORMED`/`E_LANE_DOUBLE_CLAIM`) are unrepresentable in this
//! format by construction, not merely fixed.
//!
//! Schema: `base-template/.claude/workflows/lane.schema.json`, governed by D71. This
//! module's [`LaneDirectives`]/[`LaneBudget`] and the cross-repo derived artifact types
//! ([`DerivedBlockPosition`], [`LaneSegmentsArtifact`]) keep their exact pre-existing
//! shape — `core/engine-rs`'s `chain.rs` re-declares `LaneDirectives`/`LaneBudget` as
//! `deny_unknown_fields` mirrors, and `emit-state`'s [`LANE_SEGMENTS_ARTIFACT`] is a
//! frozen cross-repo contract. Do not change their field sets here without updating
//! `engine-rs` too.
//!
//! # Both roadmap layouts
//!
//! A roadmap directory is either the current layout, `planning/roadmaps/<slug>/`, or the
//! legacy layout, `planning/<slug>/`. Both are discovered. Per `/begin-orchestration`
//! Step 1C, a slug present under **both** locations at once is ambiguous — resolving it by
//! silent preference would let a derivation run against the wrong lane, so it is reported
//! as an error diagnostic instead of picked for the caller.
//!
//! # `deferred-blocks.txt` is out of scope on purpose
//!
//! It holds blocks cut by operator decision and is the *only* place that decision is
//! encoded — nothing in `state.json` mirrors it. The `lane-<name>.json` glob must never
//! widen to catch it.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::Diagnostic;
use crate::brain::state::{BlockDep, BlockedBy, StateFile, StateSource, TrackBlock};

/// Name of the directory holding roadmaps under the current (non-legacy) layout.
const ROADMAPS_DIR: &str = "roadmaps";

/// Directory names that are never roadmap slugs, even though they sit directly under
/// `planning/` alongside legacy roadmap directories.
const NON_ROADMAP_DIR_NAMES: &[&str] = &["archive", "decisions", "artifacts"];

/// One block reference inside a lane record: the ID plus the authored ownership the
/// record's `blocks[]` entry carries — the repo it runs in, and the roadmap it was
/// originally allocated under (`origin_roadmap`), which retires `E_LANE_DOUBLE_CLAIM`
/// by construction (see the module doc).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneBlockRef {
    pub id: String,
    /// 1-based position of this block within the record's `blocks[]` array — array
    /// order IS chain order, so this is also a usable diagnostic location even though
    /// nothing here is a source *line* any more.
    pub line: usize,
    /// The roadmap this block was originally allocated under, per the record's
    /// authored, required `origin_roadmap` field. Always `Some(..)` for a
    /// successfully-deserialized record — carried as `Option` to match
    /// [`DerivedBlockPosition::origin_roadmap`], which this value flows into
    /// unchanged.
    pub origin_roadmap: Option<String>,
    /// The repo slug this block runs in, authored per-block in the record — never
    /// inherited from a lane-level default (a lane is not single-repo in this corpus).
    pub repo: String,
}

/// One discovered, parsed `lane-<name>.json` record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneFile {
    /// The owning roadmap's slug — the name of the directory directly containing this
    /// lane record, whether that directory sits under `planning/roadmaps/` or legacy
    /// `planning/<slug>/`.
    pub roadmap: String,
    /// The lane name: the filename with the `lane-` prefix and `.json` suffix stripped
    /// (`lane-substrate.json` → `substrate`).
    pub lane: String,
    /// Absolute path to the file on disk.
    pub path: PathBuf,
    /// The ordered block list, exactly as authored in the record's `blocks[]` array.
    /// Array order is execution order and is preserved exactly — never sorted,
    /// deduped, or normalised.
    pub blocks: Vec<LaneBlockRef>,
    /// This lane's structured directives (`MV.ticket.lane-file-structured-directives`
    /// Task 1), or `None` when the record declares none of `held_until`, `budget` or
    /// `exclusive_repos` — absence, never a defaulted or empty-but-present value. See
    /// [`LaneDirectives`] for the grammar.
    pub directives: Option<LaneDirectives>,
}

/// A lane's structured directives — machine-readable declarations of a hold, a heavy/light
/// resource budget, and cross-lane repo exclusivity. Authored directly as JSON keys on the
/// lane record (`held_until`, `budget`, `exclusive_repos`) per
/// `base-template/.claude/workflows/lane.schema.json`.
///
/// Every field is `Option`, absent unless the lane record declares it, and the whole
/// struct is itself wrapped in `Option` on [`LaneFile::directives`]: a lane declaring
/// **none** of the three directives produces `directives: None` there, never
/// `Some(LaneDirectives { held_until: None, budget: None, exclusive_repos: None })`.
/// Absence must read as "unspecified", never as "unconstrained" — a caller that only
/// checks `is_some()` on the wrapping `Option` gets the right answer either way, but a
/// caller that matches on individual fields must not be able to mistake "declared
/// nothing" for "declared an empty constraint".
///
/// **Hand-mirrored contract**: `core/engine-rs`'s
/// `crates/engine-core/src/workflows/orchestration/chain.rs` re-declares this type as a
/// reading-side mirror, with no shared dependency to enforce the mirror — so the two must be
/// kept in lockstep by hand. That mirror is deliberately **not** `deny_unknown_fields`
/// (`EN.12.E` task 1): an unrecognised key there parses and is dropped rather than
/// hard-failing the whole lane-segments read, so mev shipping a new field first degrades
/// gracefully instead of breaking every un-rebuilt engine on every lane segment. Adding a
/// field here is therefore safe; **renaming or removing one is not**, and still requires
/// updating that file.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct LaneDirectives {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub held_until: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget: Option<LaneBudget>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclusive_repos: Option<Vec<String>>,
}

impl LaneDirectives {
    /// `true` iff none of the three fields is set — the signal the record-to-struct
    /// mapping in [`collect_lane_files_in`] uses to collapse an all-absent result down
    /// to `None` rather than an empty `Some`.
    fn is_empty(&self) -> bool {
        self.held_until.is_none() && self.budget.is_none() && self.exclusive_repos.is_none()
    }
}

/// The `budget` directive's parsed value — see [`LaneDirectives`] for the grammar.
///
/// **Frozen contract**: this type's field set and serialized shape must not change
/// without updating `core/engine-rs`'s `chain.rs`, which re-declares it as a
/// `deny_unknown_fields` mirror. `not_with` defaults to empty when the JSON record
/// omits it — an empty list here means "no additional exclusion beyond the budget
/// class itself", which is different from [`LaneDirectives::exclusive_repos`] being
/// absent entirely.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LaneBudget {
    /// `true` for a heavy lane, `false` for light.
    pub heavy: bool,
    /// Repo slugs this lane's budget must not run concurrently beside.
    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub not_with: Vec<String>,
}

/// Error code on a diagnostic produced when a `lane-<name>.json` file fails to
/// deserialize against `base-template/.claude/workflows/lane.schema.json` — an unknown
/// top-level or block-level key (`deny_unknown_fields`), a missing required field, or
/// invalid JSON. Named and never silently skipped: a malformed record still ends up
/// nowhere in the derived output, so a diagnostic is the only way its author finds out.
const E_LANE_RECORD_MALFORMED: &str = "E_LANE_RECORD_MALFORMED";

/// Warning code on a diagnostic produced when a roadmap directory (identified by a
/// `roadmap.md` at its top level) has no `lane-<name>.json` record at all. This is the
/// silent-miss case this whole initiative exists to catch: a directory in this shape
/// contributes zero blocks to segmentation, and without this diagnostic that is
/// indistinguishable from "this roadmap has no lanes worth reporting" — until
/// `HQ.8.A` converts the fleet's legacy `.txt` lane substrates, this is true of every
/// roadmap directory, so it is a warning, never an error (an error would red-gate the
/// whole fleet on day one).
const W_LANE_DIR_NO_RECORD: &str = "W_LANE_DIR_NO_RECORD";

/// Raw on-disk deserialize shape for one `lane-<name>.json` record, mirroring
/// `base-template/.claude/workflows/lane.schema.json` field-for-field —
/// `deny_unknown_fields` so an unrecognised key is a loud error rather than silently
/// ignored. This is the authored shape only; [`collect_lane_files_in`] maps it onto
/// [`LaneFile`]/[`LaneBlockRef`]/[`LaneDirectives`] before it leaves this module — no
/// other code in the crate should ever see a [`LaneRecord`] directly.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct LaneRecord {
    #[allow(dead_code)]
    // validated against the containing directory's slug at some future point; not required for Task 2's contract
    lane: String,
    #[allow(dead_code)]
    // matches the containing directory's slug (`collect_lane_files_in` uses the directory, not this field)
    roadmap: String,
    /// Optional lane-level default repo — carries no meaning for `blocks[]` (each
    /// block authors its own `repo` regardless), present only so a single-repo lane
    /// can name itself once. Not otherwise consumed by this module.
    #[serde(default)]
    #[allow(dead_code)]
    repo: Option<String>,
    blocks: Vec<LaneRecordBlock>,
    #[serde(default)]
    budget: Option<LaneBudget>,
    #[serde(default)]
    held_until: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    // authored lane metadata with no state.json representation; not consumed by derivation
    isolation: Option<String>,
    #[serde(default)]
    exclusive_repos: Option<Vec<String>>,
    #[serde(default)]
    #[allow(dead_code)]
    // authored lane metadata with no state.json representation; not consumed by derivation
    spec_source: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    // authored lane metadata with no state.json representation; not consumed by derivation
    cut_blocks: Option<Vec<String>>,
    /// The lane's authored briefing — the prose that used to live in the `lane-*.txt`
    /// header comments.
    ///
    /// **This field exists so that prose survives conversion, and its absence was a P0.**
    /// `LaneRecord` is `deny_unknown_fields`, so before this field existed a record
    /// carrying `notes` failed to deserialize and contributed to no derived artifact —
    /// silently. Measured 2026-08-21 against the installed binary, same record, one key
    /// different: with `notes`, `mev lanes --json` printed `{"segments": []}` and exited
    /// **0**; without it, one segment. A converter that moved 70 lane files' briefings
    /// into this field would therefore have produced a clean-looking, zero-exit, entirely
    /// empty lane surface — indistinguishable from "this corpus has no lanes".
    ///
    /// Authored lane metadata with no `state.json` representation and not consumed by
    /// derivation; it is carried so the briefing has a home, per D71 and base-template's
    /// `BT.ticket.lane-schema-has-no-home-for-the-briefing`. Deliberately lane-level only:
    /// per-block prose belongs in `planning/blocks/<ID>.json`, because no SDLC engine has
    /// ever opened a lane file, and two per-block prose homes means the next author picks
    /// the wrong one half the time.
    #[serde(default)]
    #[allow(dead_code)]
    notes: Option<String>,
}

/// Raw on-disk deserialize shape for one `blocks[]` entry — see [`LaneRecord`].
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct LaneRecordBlock {
    id: String,
    origin_roadmap: String,
    repo: String,
}

/// Discover every live `lane-<name>.json` record under `root/planning/roadmaps/<slug>/`
/// and legacy `root/planning/<slug>/`, excluding anything under an `archive/` directory
/// at any depth and any file or directory whose name starts with `_` (the corpus-wide
/// ephemeral/debug convention). Symlinks are followed — every `planning/` in this
/// fleet is itself a symlink into a `_planning/` vault, and a symlink-blind walk would
/// silently return a subset while looking successful.
///
/// Returns the discovered, parsed lane records plus diagnostics for structural
/// problems: a roadmap slug claimed by both layouts at once, or a record that fails to
/// deserialize (never silently skipped — see [`E_LANE_RECORD_MALFORMED`]).
pub fn discover_lane_files(root: &Path) -> (Vec<LaneFile>, Vec<Diagnostic>) {
    let planning_dir = root.join("planning");
    let mut lane_files = Vec::new();
    let mut diags = Vec::new();

    if !planning_dir.is_dir() {
        return (lane_files, diags);
    }

    let mut roadmaps_slugs: HashSet<String> = HashSet::new();
    let mut legacy_slugs: HashSet<String> = HashSet::new();

    // Current layout: planning/roadmaps/<slug>/lane-<name>.json
    let roadmaps_dir = planning_dir.join(ROADMAPS_DIR);
    if roadmaps_dir.is_dir() {
        for (slug, slug_dir) in child_dirs(&roadmaps_dir) {
            roadmaps_slugs.insert(slug.clone());
            let found_before = lane_files.len();
            collect_lane_files_in(&slug_dir, &slug, &mut lane_files, &mut diags);
            if lane_files.len() == found_before {
                warn_if_roadmap_dir_has_no_lane_record(&slug_dir, &mut diags);
            }
        }
    }

    // Legacy layout: planning/<slug>/lane-<name>.json — every direct child of
    // planning/ except roadmaps/ itself and the non-roadmap directories that live
    // alongside roadmap dirs.
    for (slug, slug_dir) in child_dirs(&planning_dir) {
        if slug == ROADMAPS_DIR || NON_ROADMAP_DIR_NAMES.contains(&slug.as_str()) {
            continue;
        }
        // Only a directory that actually contains a lane record counts as a "legacy
        // roadmap directory" for the both-locations check below — many planning/
        // children (spec dirs, orchestration-run, etc.) are not roadmaps at all and
        // must not collide with a same-named roadmaps/ entry that has none either.
        let found_before = lane_files.len();
        collect_lane_files_in(&slug_dir, &slug, &mut lane_files, &mut diags);
        if lane_files.len() > found_before {
            legacy_slugs.insert(slug);
        } else {
            warn_if_roadmap_dir_has_no_lane_record(&slug_dir, &mut diags);
        }
    }

    let mut both: Vec<&String> = roadmaps_slugs.intersection(&legacy_slugs).collect();
    both.sort();
    for slug in both {
        diags.push(Diagnostic::error(
            &planning_dir,
            "",
            format!(
                "roadmap slug '{slug}' exists under both planning/{ROADMAPS_DIR}/{slug}/ and legacy planning/{slug}/ — ambiguous, resolve manually (never a silent preference)"
            ),
        ));
    }

    (lane_files, diags)
}

/// If `slug_dir` (a roadmap directory that yielded zero lane records) contains a
/// `roadmap.md` at its top level, push a [`W_LANE_DIR_NO_RECORD`] warning naming the
/// path. Called only when the caller has already confirmed no lane record was found
/// under `slug_dir` — this just decides whether that emptiness is worth flagging (a
/// directory with no `roadmap.md` either is not a roadmap directory at all, e.g. an
/// unrelated `planning/` child, and stays silent).
fn warn_if_roadmap_dir_has_no_lane_record(slug_dir: &Path, diags: &mut Vec<Diagnostic>) {
    if !slug_dir.join("roadmap.md").is_file() {
        return;
    }
    diags.push(Diagnostic::warning(
        slug_dir,
        W_LANE_DIR_NO_RECORD,
        format!(
            "{} has a roadmap.md but no lane-*.json record — its blocks cannot be \
             lane-segmented until one is authored",
            slug_dir.display()
        ),
    ));
}

/// Direct subdirectories of `dir`, as `(name, path)`, excluding any whose name starts
/// with `_`. Not recursive — callers walk deeper themselves where needed.
fn child_dirs(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let Ok(read) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in read.flatten() {
        // `file_type()` on a `DirEntry` follows symlinks on most platforms only for the
        // metadata call itself; use `path().is_dir()` so a `planning -> ../_planning/x`
        // style symlinked directory entry is still recognised as a directory.
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('_') {
            continue;
        }
        out.push((name, path));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Walk `slug_dir` (following symlinks) for `lane-<name>.json` files, excluding
/// anything under an `archive` path segment and any `_`-prefixed file, deserialize
/// each against [`LaneRecord`], and push a [`LaneFile`] per hit (plus a diagnostic on
/// unreadable content or a malformed record).
fn collect_lane_files_in(
    slug_dir: &Path,
    slug: &str,
    lane_files: &mut Vec<LaneFile>,
    diags: &mut Vec<Diagnostic>,
) {
    let iter = walkdir::WalkDir::new(slug_dir)
        .follow_links(true)
        .into_iter()
        .filter_entry(|e| {
            if e.depth() == 0 {
                return true;
            }
            let name = e.file_name().to_string_lossy();
            if name.starts_with('_') {
                return false;
            }
            if e.file_type().is_dir() && name == "archive" {
                return false;
            }
            true
        });

    for entry in iter {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                diags.push(Diagnostic::error(slug_dir, "", format!("walk error: {e}")));
                continue;
            }
        };
        if !entry.file_type().is_file() {
            continue;
        }
        let file_name = entry.file_name().to_string_lossy().to_string();
        if !is_lane_file_name(&file_name) {
            continue;
        }
        let path = entry.path().to_path_buf();
        let lane = lane_name_from_file(&file_name);

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                diags.push(Diagnostic::error(
                    &path,
                    "",
                    format!("could not read lane record: {e}"),
                ));
                continue;
            }
        };

        let record: LaneRecord = match serde_json::from_str(&content) {
            Ok(r) => r,
            Err(e) => {
                diags.push(Diagnostic::error(
                    &path,
                    E_LANE_RECORD_MALFORMED,
                    format!("{}: malformed lane record: {e}", path.display()),
                ));
                continue;
            }
        };

        let directives = {
            let d = LaneDirectives {
                held_until: record.held_until.clone(),
                budget: record.budget.clone(),
                exclusive_repos: record.exclusive_repos.clone(),
            };
            if d.is_empty() { None } else { Some(d) }
        };

        let blocks: Vec<LaneBlockRef> = record
            .blocks
            .iter()
            .enumerate()
            .map(|(i, b)| LaneBlockRef {
                id: b.id.clone(),
                line: i + 1,
                origin_roadmap: Some(b.origin_roadmap.clone()),
                repo: b.repo.clone(),
            })
            .collect();

        lane_files.push(LaneFile {
            roadmap: slug.to_string(),
            lane,
            path,
            blocks,
            directives,
        });
    }
}

/// `true` iff `file_name` matches the `lane-<name>.json` glob and is not excluded by
/// the leading-`_` debug-file convention. `deferred-blocks.txt` deliberately does not
/// match this (no `lane-` prefix, and it stays `.txt`) — it is out of scope for this
/// module by design.
fn is_lane_file_name(file_name: &str) -> bool {
    !file_name.starts_with('_')
        && file_name.starts_with("lane-")
        && file_name.ends_with(".json")
        && file_name.len() > "lane-".len() + ".json".len()
}

/// `lane-substrate.json` → `substrate`.
fn lane_name_from_file(file_name: &str) -> String {
    file_name
        .strip_prefix("lane-")
        .and_then(|s| s.strip_suffix(".json"))
        .unwrap_or(file_name)
        .to_string()
}

// ---------------------------------------------------------------------------
// Ownership and segmentation — MV.13.A Task 2, simplified by MV.17.A Task 2
// ---------------------------------------------------------------------------

/// Corpus-wide index from a literal block-ID string to every repo slug that owns a
/// block by that exact ID, built from `state.json`'s `tracks[].blocks[]` across every
/// loaded file. State-graph keys are `repo:id` (see [`crate::brain::block_graph`]), so
/// this index groups by repo *before* collapsing to a single owner — a bare ID that
/// happens to be authored under two repos is a legitimate corpus state, not a bug.
///
/// No longer used to resolve lane-block ownership (each block authors its own `repo`
/// directly — see [`LaneBlockRef`]); kept for [`unresolved_owner_diagnostics`], which
/// uses it to check an authored `repo` against the set of repos the corpus actually
/// knows, and for any other consumer that needs a corpus-wide id→repo index.
pub type OwnerIndex = HashMap<String, Vec<String>>;

/// Build the corpus-wide [`OwnerIndex`] from every loaded `state.json`.
///
/// `files` is the same `(StateSource, StateFile)` pairing [`crate::brain::state::discover_state_files`]
/// plus `load_state` produce for the whole corpus — this function does no discovery or
/// I/O of its own.
pub fn build_owner_index(files: &[(StateSource, StateFile)]) -> OwnerIndex {
    let mut index: OwnerIndex = HashMap::new();
    for (src, file) in files {
        for track in &file.tracks {
            for block in &track.blocks {
                let repos = index.entry(block.id.clone()).or_default();
                if !repos.iter().any(|r| r == &src.repo_slug) {
                    repos.push(src.repo_slug.clone());
                }
            }
        }
    }
    index
}

/// Resolve a bare block ID to its single owning repo slug via `index`.
///
/// Returns `None` — never a guess — when the ID resolves to **zero** repos (unknown to
/// the corpus) or to **more than one** (a bare ID legitimately reused across repos).
/// No longer used for lane-block ownership (see [`OwnerIndex`]'s doc); kept exported
/// for other id→repo lookups.
pub fn resolve_owner<'a>(index: &'a OwnerIndex, id: &str) -> Option<&'a str> {
    match index.get(id) {
        Some(repos) if repos.len() == 1 => Some(repos[0].as_str()),
        _ => None,
    }
}

/// One block inside a [`LaneSegment`], carrying its position within that segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneSegmentBlock {
    pub id: String,
    /// 1-based position within the owning [`LaneFile`]'s `blocks[]` array, carried
    /// through from [`LaneBlockRef::line`].
    pub line: usize,
    /// 0-based index of this block within its segment — the second half of the
    /// `{segment, position}` pair. Chosen zero-based to match [`LaneSegment::segment`]
    /// and every other zero-based index already in this derivation (`topo_index`,
    /// vec indices generally); documented once here, not re-chosen per call site.
    pub position: usize,
    /// The block's authored `origin_roadmap`, carried through from
    /// [`LaneBlockRef::origin_roadmap`] unchanged.
    pub origin_roadmap: Option<String>,
}

/// One contiguous run of a lane's blocks all owned by the same repo — the `(repo,
/// chain)` segment this task derives. **Derived at emit time; never authored** — a lane
/// record stays the authoring surface and nothing here is written back into one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneSegment {
    pub repo: String,
    /// 0-based index of this segment within its lane.
    pub segment: usize,
    /// Ordered, in file order — never sorted, deduped, or normalised.
    pub blocks: Vec<LaneSegmentBlock>,
    /// The owning lane's structured directives (see [`LaneDirectives`]), carried onto
    /// every segment of that lane unchanged — `None` when the lane declared none.
    /// [`segment_lane_blocks`] always produces `None` here (it has no [`LaneFile`] to read
    /// them from); [`segment_lane_file_segments`] fills this in from the lane file it
    /// segments.
    pub directives: Option<LaneDirectives>,
}

impl LaneSegment {
    /// The segment's first block — the block the frontier computation reads.
    pub fn head(&self) -> Option<&LaneSegmentBlock> {
        self.blocks.first()
    }
}

/// Segment an ordered block list into `(repo, chain)` runs by cutting a new segment at
/// every change in each block's own authored `repo` (see [`LaneBlockRef::repo`]) — no
/// external resolver needed any more, since ownership is authored on the block itself
/// rather than derived from the corpus.
///
/// Order is preserved exactly, within and across segments — a repo appearing twice
/// **non-contiguously** (e.g. `A, B, A`) yields **three** segments, never one merged
/// segment; that non-contiguous case is what separates this from a plain `group_by`.
pub fn segment_lane_blocks(blocks: &[LaneBlockRef]) -> Vec<LaneSegment> {
    let mut segments: Vec<LaneSegment> = Vec::new();
    for block in blocks {
        let starts_new_segment = match segments.last() {
            Some(seg) => seg.repo != block.repo,
            None => true,
        };
        if starts_new_segment {
            segments.push(LaneSegment {
                repo: block.repo.clone(),
                segment: segments.len(),
                blocks: Vec::new(),
                directives: None,
            });
        }
        let seg = segments
            .last_mut()
            .expect("just pushed or matched an existing segment above");
        let position = seg.blocks.len();
        seg.blocks.push(LaneSegmentBlock {
            id: block.id.clone(),
            line: block.line,
            position,
            origin_roadmap: block.origin_roadmap.clone(),
        });
    }
    segments
}

/// One block, positioned within its lane after ownership-based segmentation —
/// `{roadmap, lane, segment, position}` plus its authored `origin_roadmap`, the
/// per-block derived record this task produces. Derived at emit time; authored
/// nowhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneBlockPosition {
    pub roadmap: String,
    pub lane: String,
    pub repo: String,
    pub id: String,
    pub line: usize,
    pub segment: usize,
    pub position: usize,
    pub origin_roadmap: Option<String>,
    /// The owning lane's structured directives (see [`LaneDirectives`]), carried through
    /// from the [`LaneSegment`] this block belonged to — `None` when the lane declared
    /// none.
    pub directives: Option<LaneDirectives>,
}

/// Build a `"repo:id"` -> [`TrackBlock`] index across every loaded `state.json`, the
/// local equivalent of `frontier.rs`'s private `track_block_index` — kept as a separate
/// copy rather than made `pub` there, per this task's scope.
fn dependency_block_index(files: &[(StateSource, StateFile)]) -> HashMap<String, &TrackBlock> {
    let mut index = HashMap::new();
    for (src, file) in files {
        for track in &file.tracks {
            for block in &track.blocks {
                let key = format!("{}:{}", src.repo_slug, block.id);
                index.insert(key, block);
            }
        }
    }
    index
}

/// Further split one repo-grouped [`LaneSegment`] at every block (other than the
/// first) that carries a real, unmet dependency — an `Operator`/`Approval`/`External`
/// edge, a `Block` edge to an open target in a different repo, or a `Block` edge to an
/// open target present in no lane file anywhere. This is what makes a long same-repo
/// run report its real internal barriers instead of hiding them behind one segment
/// (see this task's spec).
///
/// A `Block` edge whose target is already `closed`, or whose target already appears
/// **earlier in the sub-segment currently being built**, never splits — lane order is
/// load-bearing: a dependency the lane itself will have already closed by the time
/// execution reaches this block is not a real blocker. `seen` resets to empty at the
/// start of each new sub-segment for exactly this reason.
///
/// **Documented non-split case, deliberately out of scope**: a same-repo, not-yet-closed
/// dependency whose target appears in *some* lane file (just not earlier in this one)
/// does not split here — the correct behaviour for that case is unspecified by this
/// task, so this function does not guess at it.
pub fn split_segment_on_unmet_dependencies(
    seg: LaneSegment,
    loaded: &[(StateSource, StateFile)],
    all_lane_files: &[LaneFile],
) -> Vec<LaneSegment> {
    if seg.blocks.len() <= 1 {
        return vec![seg];
    }

    let block_index = dependency_block_index(loaded);
    let global_status = crate::brain::emit::global_status_map(loaded);
    let known_lane_keys: HashSet<String> = all_lane_files
        .iter()
        .flat_map(|lf| lf.blocks.iter().map(|b| format!("{}:{}", b.repo, b.id)))
        .collect();

    let mut out: Vec<LaneSegment> = Vec::new();
    let mut current: Vec<LaneSegmentBlock> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for (i, block) in seg.blocks.into_iter().enumerate() {
        let block_key = format!("{}:{}", seg.repo, block.id);

        let should_split = i > 0
            && block_index.get(&block_key).is_some_and(|tb| {
                tb.depends_on.iter().any(|dep| match dep {
                    BlockedBy::Operator(_) | BlockedBy::Approval(_) | BlockedBy::External(_) => {
                        true
                    }
                    BlockedBy::Block(BlockDep { repo, id, .. }) => {
                        let target_key = format!("{repo}:{id}");
                        let target_status = global_status
                            .get(&target_key)
                            .cloned()
                            .flatten()
                            .unwrap_or_else(|| "open".to_string());
                        if target_status == "closed" || seen.contains(&target_key) {
                            false
                        } else if repo != &seg.repo {
                            true
                        } else {
                            !known_lane_keys.contains(&target_key)
                        }
                    }
                })
            });

        if should_split && !current.is_empty() {
            out.push(LaneSegment {
                repo: seg.repo.clone(),
                segment: 0, // renumbered by the caller once every split is known
                blocks: std::mem::take(&mut current),
                directives: seg.directives.clone(),
            });
            seen.clear();
        }

        seen.insert(block_key);
        current.push(block);
    }

    if !current.is_empty() {
        out.push(LaneSegment {
            repo: seg.repo.clone(),
            segment: 0,
            blocks: current,
            directives: seg.directives,
        });
    }

    out
}

/// Segment one [`LaneFile`]'s blocks (via [`segment_lane_blocks`]), further split each
/// resulting repo-grouped segment at every mid-run unmet dependency (via
/// [`split_segment_on_unmet_dependencies`]), then stamp the lane's structured
/// directives (see [`LaneDirectives`]) onto every resulting [`LaneSegment`] and
/// renumber `segment`/`position` sequentially, 0-based — the directives describe the
/// whole lane, so every segment of it carries the same value (`None` when the lane
/// declared none), never a per-segment default.
pub fn segment_lane_file_segments(
    lane_file: &LaneFile,
    loaded: &[(StateSource, StateFile)],
    all_lane_files: &[LaneFile],
) -> Vec<LaneSegment> {
    let mut segments: Vec<LaneSegment> = segment_lane_blocks(&lane_file.blocks)
        .into_iter()
        .flat_map(|seg| split_segment_on_unmet_dependencies(seg, loaded, all_lane_files))
        .collect();

    for (seg_idx, seg) in segments.iter_mut().enumerate() {
        seg.segment = seg_idx;
        seg.directives = lane_file.directives.clone();
        for (pos_idx, block) in seg.blocks.iter_mut().enumerate() {
            block.position = pos_idx;
        }
    }
    segments
}

/// Segment one [`LaneFile`]'s blocks (via [`segment_lane_file_segments`]) and flatten the
/// result into one [`LaneBlockPosition`] per block, each carrying its owning lane
/// file's `roadmap`/`lane`.
pub fn segment_lane_file(
    lane_file: &LaneFile,
    loaded: &[(StateSource, StateFile)],
    all_lane_files: &[LaneFile],
) -> Vec<LaneBlockPosition> {
    segment_lane_file_segments(lane_file, loaded, all_lane_files)
        .into_iter()
        .flat_map(|seg| {
            let LaneSegment {
                repo,
                segment,
                blocks,
                directives,
            } = seg;
            let roadmap = lane_file.roadmap.clone();
            let lane = lane_file.lane.clone();
            blocks.into_iter().map(move |b| LaneBlockPosition {
                roadmap: roadmap.clone(),
                lane: lane.clone(),
                repo: repo.clone(),
                id: b.id,
                line: b.line,
                segment,
                position: b.position,
                origin_roadmap: b.origin_roadmap,
                directives: directives.clone(),
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// An authored repo the corpus does not know — never a silent miss
// ---------------------------------------------------------------------------

/// Emit a diagnostic for every block in `lane_file` whose authored `repo` (see
/// [`LaneBlockRef::repo`]) names a repo `index` has never seen own any block —
/// i.e. a typo'd or retired repo slug in an authored lane record. Ownership itself is
/// never resolved from `index` any more (see [`OwnerIndex`]'s doc); this is purely a
/// sanity check on the authored value.
///
/// **Warning, never error, and never aborts derivation.** A hard error here would
/// red-gate the whole corpus over a repo the local `state.json` set simply hasn't
/// loaded yet (e.g. a partial checkout) — segmentation proceeds over every block
/// regardless, this function only reports what looks wrong and why.
pub fn unresolved_owner_diagnostics(lane_file: &LaneFile, index: &OwnerIndex) -> Vec<Diagnostic> {
    let known_repos: HashSet<&str> = index
        .values()
        .flat_map(|repos| repos.iter().map(String::as_str))
        .collect();

    let mut diags = Vec::new();
    for block in &lane_file.blocks {
        if known_repos.contains(block.repo.as_str()) {
            continue;
        }
        diags.push(Diagnostic::warning(
            &lane_file.path,
            block.line.to_string(),
            format!(
                "lane block '{}' names repo '{}', which the corpus does not know",
                block.id, block.repo
            ),
        ));
    }
    diags
}

// ---------------------------------------------------------------------------
// Derived positions — MV.13.A Task 4, simplified by MV.17.A Task 2
// ---------------------------------------------------------------------------

/// One block's final derived position, corpus-wide: `{roadmap, lane, segment,
/// position}` plus `origin_roadmap` (the block's authored owning roadmap, if any).
///
/// **Frozen contract**: this type's field set and serialized shape must not change
/// without updating `core/engine-rs`'s `chain.rs`, which links against this crate as a
/// path dependency, and without checking `emit-state`'s
/// [`LANE_SEGMENTS_ARTIFACT`] consumers.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DerivedBlockPosition {
    pub roadmap: String,
    pub lane: String,
    pub repo: String,
    pub id: String,
    pub line: usize,
    pub segment: usize,
    pub position: usize,
    pub origin_roadmap: Option<String>,
    /// The owning lane's structured directives (`MV.ticket.lane-file-structured-directives`
    /// Task 3), widening the `{roadmap, lane, segment, position}` shape rather than
    /// replacing it. Omitted entirely (no key, never `null`) when the lane declared none —
    /// this keeps [`LaneSegmentsArtifact`] byte-identical for every lane that declares no
    /// directives.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub directives: Option<LaneDirectives>,
}

/// Derive `{roadmap, lane, segment, position}` for every block across `lane_files`.
///
/// Every block in every discovered lane file renders — there is no double-claim
/// exclusion any more (see the module doc: a required per-block `origin_roadmap` makes
/// an unannotated double claim unrepresentable, so a block appearing in more than one
/// lane file simply renders once per appearance, each with its own authored
/// `origin_roadmap`). The `Vec<Diagnostic>` return is kept for API stability with
/// [`plan_lane_segments`] and its sibling planners; it is always empty today.
pub fn derive_lane_positions(
    lane_files: &[LaneFile],
    loaded: &[(StateSource, StateFile)],
) -> (Vec<DerivedBlockPosition>, Vec<Diagnostic>) {
    let mut out = Vec::new();
    for lf in lane_files {
        for p in segment_lane_file(lf, loaded, lane_files) {
            out.push(DerivedBlockPosition {
                roadmap: p.roadmap,
                lane: p.lane,
                repo: p.repo,
                id: p.id,
                line: p.line,
                segment: p.segment,
                position: p.position,
                origin_roadmap: p.origin_roadmap,
                directives: p.directives,
            });
        }
    }
    (out, Vec::new())
}

// ---------------------------------------------------------------------------
// Wired into `emit-state` — MV.13.A Task 5
// ---------------------------------------------------------------------------

/// Relative path (from the brain root) of the derived lane-segments artifact this
/// module's planner writes. A cross-repo, corpus-wide derivation — not one repo's
/// surface — so it is written unconditionally by [`plan_lane_segments`] rather than
/// being one of the four scoped targets [`crate::brain::config::ScopeDependencySet`]
/// recognises; `emit_state`'s `--scope <repo>` narrowing does not apply to it.
pub const LANE_SEGMENTS_ARTIFACT: &str = "planning/lane-segments.json";

/// The full corpus-wide derivation this module produces, serialized as-is —
/// `mev emit-state`'s JSON artifact at [`LANE_SEGMENTS_ARTIFACT`].
///
/// **Frozen contract**: see [`DerivedBlockPosition`]'s doc.
#[derive(Debug, Clone, serde::Serialize)]
struct LaneSegmentsArtifact {
    /// Every lane file's derived block positions, corpus-wide, in the same order
    /// [`derive_lane_positions`] returns them — lane-file discovery order, then file
    /// order within each lane. Never re-sorted here.
    blocks: Vec<DerivedBlockPosition>,
}

/// Plan the [`LANE_SEGMENTS_ARTIFACT`] write: discover every live lane record under
/// `root` and derive `{roadmap, lane, segment, position}` for every block — assembled
/// into one [`EmitPlan`] for `emit_state` to apply alongside its other planners.
///
/// Diagnostics carried on the returned plan are the union of:
/// - [`discover_lane_files`]'s structural diagnostics (e.g. a slug claimed by both
///   roadmap layouts at once, or a record that failed to deserialize);
/// - [`unresolved_owner_diagnostics`] for every block in every lane file, naming an
///   authored `repo` the corpus does not know.
///
/// No `EmitAction` is planned when zero lane files are discovered (an empty corpus, or
/// `root` has no `planning/` at all) — nothing to derive, nothing to write.
pub fn plan_lane_segments(
    root: &Path,
    loaded: &[(StateSource, StateFile)],
) -> crate::brain::emit::EmitPlan {
    use crate::brain::emit::{EmitAction, EmitPlan};

    let mut plan = EmitPlan::default();

    let (lane_files, discover_diags) = discover_lane_files(root);
    plan.diagnostics.extend(discover_diags);

    if lane_files.is_empty() {
        return plan;
    }

    let owner_index = build_owner_index(loaded);
    for lf in &lane_files {
        plan.diagnostics
            .extend(unresolved_owner_diagnostics(lf, &owner_index));
    }

    let (blocks, derive_diags) = derive_lane_positions(&lane_files, loaded);
    plan.diagnostics.extend(derive_diags);

    let block_count = blocks.len();
    let artifact = LaneSegmentsArtifact { blocks };
    let new_content = match serde_json::to_string_pretty(&artifact) {
        Ok(mut s) => {
            s.push('\n');
            s
        }
        Err(e) => {
            plan.diagnostics.push(Diagnostic::error(
                root,
                "E_EMIT_LANE_SEGMENTS_SERIALIZE",
                format!("failed to serialize lane-segments artifact: {e}"),
            ));
            return plan;
        }
    };

    plan.actions.push(EmitAction {
        path: root.join(LANE_SEGMENTS_ARTIFACT),
        new_content,
        note: format!(
            "derived lane segments for {block_count} block(s) across {} lane file(s)",
            lane_files.len()
        ),
    });

    plan
}

// ---------------------------------------------------------------------------
// Program-epic membership feed — `MV.13.D` Task 3
// ---------------------------------------------------------------------------

/// Extract the roadmap slug an [`crate::brain::state::Epic`]'s `plan` document names,
/// if it points at a roadmap: `.../<slug>/roadmap.md` → `Some("<slug>")`. Any other
/// filename (e.g. an area epic's `core/planning/epics/<slug>.md`) returns `None` — an
/// area has no roadmap to match against, and this deliberately does not fall back to
/// guessing a slug from the last path segment.
pub fn roadmap_slug_from_plan_path(plan: &str) -> Option<String> {
    let path = Path::new(plan);
    if path.file_name().is_some_and(|f| f == "roadmap.md") {
        path.parent()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().to_string())
    } else {
        None
    }
}

/// Derive every lane-file block's position, corpus-wide, grouped by its executing
/// roadmap slug — the feed [`crate::brain::emit::epic_members_resolved`] (`MV.13.D`
/// Task 3) consumes for `kind: program` epics.
///
/// Each roadmap's `Vec` preserves [`derive_lane_positions`]'s own output order exactly
/// (lane-file discovery order, then each file's segment/position order) — `segment` and
/// `position` are indices local to one lane file, not a corpus-wide sort key, so
/// re-sorting by them across lane files would silently interleave unrelated files'
/// blocks.
///
/// Independent derivation from [`plan_lane_segments`] (not a read of the
/// [`LANE_SEGMENTS_ARTIFACT`] it writes) because `emit_state` plans epic boards and
/// sequence tables *before* it plans the lane-segments artifact write — see
/// `src/lib.rs`'s planner ordering comment. Diagnostics are intentionally discarded
/// here: the identical discovery-and-derivation runs again in [`plan_lane_segments`]
/// later in the same `emit-state` invocation and reports them there; surfacing the same
/// diagnostic twice in one run would double-count it.
pub fn derive_program_membership(
    root: &Path,
    files: &[(StateSource, StateFile)],
) -> HashMap<String, Vec<DerivedBlockPosition>> {
    let (lane_files, _discover_diags) = discover_lane_files(root);
    if lane_files.is_empty() {
        return HashMap::new();
    }

    let (blocks, _derive_diags) = derive_lane_positions(&lane_files, files);

    let mut by_roadmap: HashMap<String, Vec<DerivedBlockPosition>> = HashMap::new();
    for b in blocks {
        by_roadmap.entry(b.roadmap.clone()).or_default().push(b);
    }
    by_roadmap
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, rel: &str, content: &str) {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    fn simple_lane_json(lane: &str, roadmap: &str, blocks: &[(&str, &str, &str)]) -> String {
        // blocks: (id, origin_roadmap, repo)
        let blocks_json: Vec<String> = blocks
            .iter()
            .map(|(id, origin_roadmap, repo)| {
                format!(r#"{{"id":"{id}","origin_roadmap":"{origin_roadmap}","repo":"{repo}"}}"#)
            })
            .collect();
        format!(
            r#"{{"lane":"{lane}","roadmap":"{roadmap}","blocks":[{}]}}"#,
            blocks_json.join(",")
        )
    }

    #[test]
    fn discover_lane_files_reads_json_and_carries_authored_origin_and_repo_in_order() {
        let dir = crate::testsupport::unique_temp_dir("mev-lane-discover-json-basic");
        write(
            &dir,
            "planning/roadmaps/alpha/lane-substrate.json",
            &simple_lane_json(
                "substrate",
                "alpha",
                &[
                    ("MV.ticket.a", "alpha", "mev"),
                    ("BT.ticket.b", "alpha", "base-template"),
                ],
            ),
        );

        let (files, diags) = discover_lane_files(&dir);
        assert!(diags.is_empty(), "expected no diagnostics, got {diags:?}");
        assert_eq!(files.len(), 1, "expected one lane file, got {files:?}");

        let f = &files[0];
        assert_eq!(f.roadmap, "alpha");
        assert_eq!(
            f.lane, "substrate",
            "lane_name_from_file must strip lane- and .json"
        );
        assert_eq!(f.blocks.len(), 2);
        assert_eq!(f.blocks[0].id, "MV.ticket.a");
        assert_eq!(f.blocks[0].repo, "mev");
        assert_eq!(f.blocks[0].origin_roadmap, Some("alpha".to_string()));
        assert_eq!(f.blocks[0].line, 1);
        assert_eq!(f.blocks[1].id, "BT.ticket.b");
        assert_eq!(f.blocks[1].repo, "base-template");
        assert_eq!(f.blocks[1].line, 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn discover_lane_files_unknown_top_level_key_is_a_loud_deserialization_error() {
        let dir = crate::testsupport::unique_temp_dir("mev-lane-discover-unknown-key");
        let fixture = std::fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/lane_json/unknown_key_lane.json"),
        )
        .expect("fixture must exist");
        write(
            &dir,
            "planning/roadmaps/alpha/lane-docs-sync.json",
            &fixture,
        );

        let (files, diags) = discover_lane_files(&dir);
        assert!(
            files.is_empty(),
            "a record with an unknown top-level key must never be silently accepted, got {files:?}"
        );
        assert_eq!(
            diags.len(),
            1,
            "expected exactly one diagnostic, got {diags:?}"
        );
        assert_eq!(diags[0].severity, crate::Severity::Error);
        assert_eq!(diags[0].locator, E_LANE_RECORD_MALFORMED);
        assert!(
            diags[0].message.contains("unexpected_top_level_key")
                || diags[0].message.to_lowercase().contains("unknown field")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Regression for the P0 found 2026-08-21, when base-template's schema ticket added a
    /// top-level `notes` field and mev's `deny_unknown_fields` reader had no such field.
    ///
    /// The failure mode is what makes this worth a dedicated test: it is SILENT. The record
    /// is rejected, `discover_lane_files` returns nothing for it, and the CLI still exits 0
    /// printing a well-formed empty result. Verified against the installed binary before the
    /// fix — same record, one key different: with `notes`, `mev lanes --json` gave
    /// `{"segments": []}` and exit 0; without it, one segment. So the assertion that matters
    /// here is not just "parses" but "yields its blocks AND raises no diagnostic".
    #[test]
    fn discover_lane_files_accepts_a_record_carrying_a_briefing() {
        let dir = crate::testsupport::unique_temp_dir("mev-lane-record-notes");
        write(
            &dir,
            "planning/roadmaps/alpha/lane-probe.json",
            r#"{
  "lane": "probe",
  "roadmap": "alpha",
  "notes": "MERGE, DO NOT INSTALL — the class of briefing this field exists to carry.",
  "blocks": [ { "id": "MV.17.A", "origin_roadmap": "alpha", "repo": "mev" } ]
}"#,
        );

        let (files, diags) = discover_lane_files(&dir);
        assert!(
            diags.is_empty(),
            "a record carrying a briefing must raise nothing, got {diags:?}"
        );
        assert_eq!(files.len(), 1, "expected the record to be discovered");
        assert_eq!(files[0].blocks.len(), 1);
        assert_eq!(files[0].blocks[0].id, "MV.17.A");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The control for the test above. `notes` is now accepted, so this pins that the field
    /// was ADDED rather than `deny_unknown_fields` being loosened — which would have made
    /// every future schema drift silent instead of loud.
    #[test]
    fn a_genuinely_unknown_key_is_still_rejected_after_notes_was_added() {
        let dir = crate::testsupport::unique_temp_dir("mev-lane-record-still-strict");
        write(
            &dir,
            "planning/roadmaps/alpha/lane-probe.json",
            r#"{
  "lane": "probe",
  "roadmap": "alpha",
  "notes": "a legitimate briefing",
  "not_a_real_field": "should still be refused",
  "blocks": [ { "id": "MV.17.A", "origin_roadmap": "alpha", "repo": "mev" } ]
}"#,
        );

        let (files, diags) = discover_lane_files(&dir);
        assert!(files.is_empty(), "deny_unknown_fields must still bite");
        assert_eq!(diags.len(), 1, "expected one diagnostic, got {diags:?}");
        assert_eq!(diags[0].locator, E_LANE_RECORD_MALFORMED);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn discover_lane_files_finds_both_layouts() {
        let dir = crate::testsupport::unique_temp_dir("mev-lane-discover-both-layouts-json");
        write(
            &dir,
            "planning/roadmaps/alpha/lane-substrate.json",
            &simple_lane_json("substrate", "alpha", &[("MV.ticket.a", "alpha", "mev")]),
        );
        write(
            &dir,
            "planning/beta/lane-gtm.json",
            &simple_lane_json("gtm", "beta", &[("OK.ticket.c", "beta", "okf-core")]),
        );

        let (files, diags) = discover_lane_files(&dir);
        assert!(diags.is_empty(), "expected no diagnostics, got {diags:?}");
        assert_eq!(files.len(), 2, "expected two lane files, got {files:?}");

        let alpha = files.iter().find(|f| f.roadmap == "alpha").unwrap();
        assert_eq!(alpha.lane, "substrate");
        assert_eq!(alpha.blocks.len(), 1);

        // 27 of 63 corpus lane files are legacy-layout — a single-layout test is not
        // evidence, so this fixture pins the legacy `planning/<slug>/` shape too.
        let beta = files.iter().find(|f| f.roadmap == "beta").unwrap();
        assert_eq!(beta.lane, "gtm");
        assert_eq!(beta.blocks.len(), 1);
        assert_eq!(beta.blocks[0].repo, "okf-core");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn discover_lane_files_excludes_archive_and_underscore_prefixed() {
        let dir = crate::testsupport::unique_temp_dir("mev-lane-discover-exclusions-json");
        write(
            &dir,
            "planning/roadmaps/alpha/archive/lane-old.json",
            &simple_lane_json("old", "alpha", &[("MV.ticket.old", "alpha", "mev")]),
        );
        write(
            &dir,
            "planning/roadmaps/alpha/_scratch/lane-debug.json",
            &simple_lane_json("debug", "alpha", &[("MV.ticket.debug", "alpha", "mev")]),
        );
        write(
            &dir,
            "planning/roadmaps/alpha/_lane-underscore.json",
            &simple_lane_json("underscore", "alpha", &[("MV.ticket.u", "alpha", "mev")]),
        );
        write(
            &dir,
            "planning/roadmaps/alpha/lane-live.json",
            &simple_lane_json("live", "alpha", &[("MV.ticket.live", "alpha", "mev")]),
        );

        let (files, diags) = discover_lane_files(&dir);
        assert!(diags.is_empty(), "expected no diagnostics, got {diags:?}");
        assert_eq!(
            files.len(),
            1,
            "expected only the live lane file, got {files:?}"
        );
        assert_eq!(files[0].lane, "live");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn discover_lane_files_deferred_blocks_txt_never_matches() {
        let dir = crate::testsupport::unique_temp_dir("mev-lane-discover-deferred-json");
        write(
            &dir,
            "planning/roadmaps/alpha/deferred-blocks.txt",
            "not a lane record\n",
        );
        write(
            &dir,
            "planning/roadmaps/alpha/lane-live.json",
            &simple_lane_json("live", "alpha", &[("MV.ticket.live", "alpha", "mev")]),
        );

        let (files, _diags) = discover_lane_files(&dir);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].lane, "live");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn discover_lane_files_both_layouts_same_slug_is_an_error() {
        let dir = crate::testsupport::unique_temp_dir("mev-lane-discover-slug-collision-json");
        write(
            &dir,
            "planning/roadmaps/alpha/lane-substrate.json",
            &simple_lane_json("substrate", "alpha", &[("MV.ticket.a", "alpha", "mev")]),
        );
        write(
            &dir,
            "planning/alpha/lane-gtm.json",
            &simple_lane_json("gtm", "alpha", &[("OK.ticket.b", "alpha", "okf-core")]),
        );

        let (files, diags) = discover_lane_files(&dir);
        // Both files are still reported (never silently dropped) — the caller decides
        // what to do with an ambiguous roadmap; this module never resolves it for them.
        assert_eq!(
            files.len(),
            2,
            "expected both files reported, got {files:?}"
        );
        assert_eq!(
            diags.len(),
            1,
            "expected exactly one collision diagnostic, got {diags:?}"
        );
        assert!(diags[0].message.contains("alpha"));
        assert_eq!(diags[0].severity, crate::Severity::Error);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn discover_lane_files_legacy_dir_without_lane_file_does_not_collide() {
        // planning/<slug>/ directories that are not roadmaps at all (a spec dir, e.g.)
        // must not be mistaken for a legacy roadmap just because roadmaps/<slug>/ exists.
        let dir = crate::testsupport::unique_temp_dir("mev-lane-discover-non-roadmap-legacy-json");
        write(
            &dir,
            "planning/roadmaps/alpha/lane-substrate.json",
            &simple_lane_json("substrate", "alpha", &[("MV.ticket.a", "alpha", "mev")]),
        );
        write(&dir, "planning/alpha/tasks.md", "not a lane record\n");

        let (files, diags) = discover_lane_files(&dir);
        assert_eq!(
            files.len(),
            1,
            "expected only the roadmaps/ lane file, got {files:?}"
        );
        assert!(
            diags.is_empty(),
            "expected no collision diagnostic, got {diags:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn discover_lane_files_roadmap_dir_with_no_lane_record_warns_naming_the_path() {
        // Task 4's silent-miss case: `roadmap.md` with no `lane-*.json` sibling must
        // not pass through discovery as a quiet zero — the exact mechanism by which
        // 27 legacy-layout lane files could disappear behind a green gate.
        let dir =
            crate::testsupport::unique_temp_dir("mev-lane-discover-no-record-warns-current-json");
        write(&dir, "planning/roadmaps/alpha/roadmap.md", "# Alpha\n");

        let (files, diags) = discover_lane_files(&dir);
        assert!(
            files.is_empty(),
            "expected zero lane records, got {files:?}"
        );
        assert_eq!(
            diags.len(),
            1,
            "expected exactly one no-record warning, got {diags:?}"
        );
        assert_eq!(diags[0].severity, crate::Severity::Warning);
        assert_eq!(diags[0].locator, W_LANE_DIR_NO_RECORD);
        let expected_path = dir.join("planning/roadmaps/alpha");
        assert!(
            diags[0]
                .message
                .contains(&expected_path.display().to_string())
                || diags[0].file == expected_path,
            "expected the warning to name {}, got {diags:?}",
            expected_path.display()
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn discover_lane_files_roadmap_dir_with_lane_record_does_not_warn() {
        let dir = crate::testsupport::unique_temp_dir(
            "mev-lane-discover-no-record-warns-has-record-json",
        );
        write(&dir, "planning/roadmaps/alpha/roadmap.md", "# Alpha\n");
        write(
            &dir,
            "planning/roadmaps/alpha/lane-substrate.json",
            &simple_lane_json("substrate", "alpha", &[("MV.ticket.a", "alpha", "mev")]),
        );

        let (files, diags) = discover_lane_files(&dir);
        assert_eq!(files.len(), 1, "expected the one lane file, got {files:?}");
        assert!(
            diags.is_empty(),
            "expected no no-record warning when a lane record exists, got {diags:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn discover_lane_files_legacy_roadmap_dir_with_no_lane_record_also_warns() {
        // Legacy layout `planning/<slug>/roadmap.md` must trip the same warning as the
        // current `planning/roadmaps/<slug>/` layout — the silent-miss risk is
        // layout-agnostic.
        let dir =
            crate::testsupport::unique_temp_dir("mev-lane-discover-no-record-warns-legacy-json");
        write(&dir, "planning/beta/roadmap.md", "# Beta\n");

        let (files, diags) = discover_lane_files(&dir);
        assert!(
            files.is_empty(),
            "expected zero lane records, got {files:?}"
        );
        assert_eq!(
            diags.len(),
            1,
            "expected exactly one no-record warning, got {diags:?}"
        );
        assert_eq!(diags[0].severity, crate::Severity::Warning);
        assert_eq!(diags[0].locator, W_LANE_DIR_NO_RECORD);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn discover_lane_files_empty_root_returns_nothing() {
        let dir = crate::testsupport::unique_temp_dir("mev-lane-discover-empty-json");
        std::fs::create_dir_all(&dir).unwrap();
        let (files, diags) = discover_lane_files(&dir);
        assert!(files.is_empty());
        assert!(diags.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn discover_lane_files_follows_planning_symlink() {
        // Every `planning/` in the live fleet is a symlink into a `_planning/` vault —
        // a symlink-blind walk must not silently return an empty result here.
        let dir = crate::testsupport::unique_temp_dir("mev-lane-discover-symlink-root-json");
        let vault = dir.join("_vault_roadmaps_alpha");
        std::fs::create_dir_all(&vault).unwrap();
        std::fs::write(
            vault.join("lane-substrate.json"),
            simple_lane_json("substrate", "alpha", &[("MV.ticket.a", "alpha", "mev")]),
        )
        .unwrap();

        std::fs::create_dir_all(dir.join("planning/roadmaps")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&vault, dir.join("planning/roadmaps/alpha")).unwrap();

        let (files, diags) = discover_lane_files(&dir);
        assert!(diags.is_empty(), "expected no diagnostics, got {diags:?}");
        assert_eq!(
            files.len(),
            1,
            "expected the symlinked lane file, got {files:?}"
        );
        assert_eq!(files[0].roadmap, "alpha");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn discover_lane_files_malformed_record_is_an_error_never_silently_skipped() {
        let dir = crate::testsupport::unique_temp_dir("mev-lane-discover-malformed-json");
        write(
            &dir,
            "planning/roadmaps/alpha/lane-broken.json",
            "{ this is not valid json",
        );
        write(
            &dir,
            "planning/roadmaps/alpha/lane-live.json",
            &simple_lane_json("live", "alpha", &[("MV.ticket.live", "alpha", "mev")]),
        );

        let (files, diags) = discover_lane_files(&dir);
        assert_eq!(
            files.len(),
            1,
            "the malformed record must never be silently skipped into a partial success, got {files:?}"
        );
        assert_eq!(files[0].lane, "live");

        let malformed: Vec<&Diagnostic> = diags
            .iter()
            .filter(|d| d.locator == E_LANE_RECORD_MALFORMED)
            .collect();
        assert_eq!(
            malformed.len(),
            1,
            "expected exactly one malformed-record diagnostic, got {diags:?}"
        );
        assert_eq!(malformed[0].severity, crate::Severity::Error);
        assert!(malformed[0].file.ends_with("lane-broken.json"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    // -- Ownership and segmentation --------------------------------------------------

    fn refs(entries: &[(&str, &str, &str)]) -> Vec<LaneBlockRef> {
        // entries: (id, origin_roadmap, repo)
        entries
            .iter()
            .enumerate()
            .map(|(i, (id, origin_roadmap, repo))| LaneBlockRef {
                id: id.to_string(),
                line: i + 1,
                origin_roadmap: Some(origin_roadmap.to_string()),
                repo: repo.to_string(),
            })
            .collect()
    }

    #[test]
    fn build_owner_index_groups_by_repo_and_flags_reuse() {
        let alpha = StateSource {
            repo_slug: "alpha".to_string(),
            abs_path: PathBuf::from("alpha/planning/state.json"),
            expected_kind: "project",
        };
        let beta = StateSource {
            repo_slug: "beta".to_string(),
            abs_path: PathBuf::from("beta/planning/state.json"),
            expected_kind: "project",
        };
        let alpha_file: StateFile = serde_json::from_str(
            r#"{"repo":"alpha","kind":"project","updated":"2026-08-14","tracks":[
                {"title":"t","blocks":[{"id":"A.one","title":"x","status":"open"}]}
            ]}"#,
        )
        .unwrap();
        let beta_file: StateFile = serde_json::from_str(
            r#"{"repo":"beta","kind":"project","updated":"2026-08-14","tracks":[
                {"title":"t","blocks":[
                    {"id":"B.one","title":"y","status":"open"},
                    {"id":"SHARED.id","title":"z","status":"open"}
                ]}
            ]}"#,
        )
        .unwrap();
        let gamma = StateSource {
            repo_slug: "gamma".to_string(),
            abs_path: PathBuf::from("gamma/planning/state.json"),
            expected_kind: "project",
        };
        let gamma_file: StateFile = serde_json::from_str(
            r#"{"repo":"gamma","kind":"project","updated":"2026-08-14","tracks":[
                {"title":"t","blocks":[{"id":"SHARED.id","title":"w","status":"open"}]}
            ]}"#,
        )
        .unwrap();

        let files = vec![(alpha, alpha_file), (beta, beta_file), (gamma, gamma_file)];
        let index = build_owner_index(&files);

        assert_eq!(resolve_owner(&index, "A.one"), Some("alpha"));
        assert_eq!(resolve_owner(&index, "B.one"), Some("beta"));
        // Reused across two repos — never guessed, resolves to None.
        assert_eq!(resolve_owner(&index, "SHARED.id"), None);
        // Unknown to the corpus entirely — also None.
        assert_eq!(resolve_owner(&index, "NOPE.id"), None);
    }

    #[test]
    fn segment_lane_blocks_cuts_at_every_ownership_change() {
        let blocks = refs(&[
            ("A.1", "alpha", "repoA"),
            ("A.2", "alpha", "repoA"),
            ("B.1", "alpha", "repoB"),
            ("C.1", "alpha", "repoC"),
            ("C.2", "alpha", "repoC"),
            ("C.3", "alpha", "repoC"),
        ]);
        let segments = segment_lane_blocks(&blocks);

        assert_eq!(segments.len(), 3, "expected 3 segments, got {segments:?}");
        assert_eq!(segments[0].repo, "repoA");
        assert_eq!(segments[0].segment, 0);
        assert_eq!(segments[0].blocks.len(), 2);
        assert_eq!(segments[0].blocks[0].position, 0);
        assert_eq!(segments[0].blocks[1].position, 1);
        assert_eq!(segments[0].head().unwrap().id, "A.1");

        assert_eq!(segments[1].repo, "repoB");
        assert_eq!(segments[1].segment, 1);
        assert_eq!(segments[1].blocks.len(), 1);

        assert_eq!(segments[2].repo, "repoC");
        assert_eq!(segments[2].segment, 2);
        assert_eq!(segments[2].blocks.len(), 3);
        assert_eq!(segments[2].blocks[2].position, 2);
    }

    #[test]
    fn segment_lane_blocks_non_contiguous_repeat_is_two_segments_not_group_by() {
        // A, B, A must yield THREE segments — the case that separates this from a
        // plain group_by, which would merge both `A` runs into one.
        let blocks = refs(&[
            ("A.1", "alpha", "repoA"),
            ("B.1", "alpha", "repoB"),
            ("A.2", "alpha", "repoA"),
        ]);
        let segments = segment_lane_blocks(&blocks);

        assert_eq!(segments.len(), 3, "expected 3 segments, got {segments:?}");
        assert_eq!(segments[0].repo, "repoA");
        assert_eq!(segments[0].blocks[0].id, "A.1");
        assert_eq!(segments[1].repo, "repoB");
        assert_eq!(segments[1].blocks[0].id, "B.1");
        assert_eq!(segments[2].repo, "repoA");
        assert_eq!(segments[2].blocks[0].id, "A.2");
        // The two `repoA` segments are NOT merged: each keeps its own 0-based
        // position, and there are two separate segment indices for the same repo.
        assert_eq!(segments[0].segment, 0);
        assert_eq!(segments[2].segment, 2);
    }

    #[test]
    fn segment_lane_file_attaches_roadmap_and_lane_to_every_position() {
        let lane_file = LaneFile {
            roadmap: "close-the-loop".to_string(),
            lane: "substrate".to_string(),
            path: PathBuf::from("planning/close-the-loop/lane-substrate.json"),
            blocks: refs(&[
                ("A.1", "close-the-loop", "repoA"),
                ("B.1", "close-the-loop", "repoB"),
                ("B.2", "close-the-loop", "repoB"),
            ]),
            directives: None,
        };
        let positions = segment_lane_file(&lane_file, &[], &[]);

        assert_eq!(positions.len(), 3);
        for p in &positions {
            assert_eq!(p.roadmap, "close-the-loop");
            assert_eq!(p.lane, "substrate");
        }
        assert_eq!(positions[0].repo, "repoA");
        assert_eq!(positions[0].segment, 0);
        assert_eq!(positions[0].position, 0);
        assert_eq!(positions[1].repo, "repoB");
        assert_eq!(positions[1].segment, 1);
        assert_eq!(positions[1].position, 0);
        assert_eq!(positions[2].repo, "repoB");
        assert_eq!(positions[2].segment, 1);
        assert_eq!(positions[2].position, 1);
    }

    // -- split_segment_on_unmet_dependencies: dependency-aware sub-segmentation -------

    /// Build one loaded `state.json` entry for repo `repo`, carrying a single
    /// `TrackBlock` `id` with authored `status` and raw JSON `depends_on` array.
    fn state_file_with_block(
        repo: &str,
        id: &str,
        status: &str,
        depends_on_json: &str,
    ) -> (StateSource, StateFile) {
        let src = StateSource {
            repo_slug: repo.to_string(),
            abs_path: PathBuf::from(format!("{repo}/planning/state.json")),
            expected_kind: "project",
        };
        let file: StateFile = serde_json::from_str(&format!(
            r#"{{"repo":"{repo}","kind":"project","updated":"2026-08-14","tracks":[
                {{"title":"t","blocks":[{{"id":"{id}","title":"x","status":"{status}","depends_on":{depends_on_json}}}]}}
            ]}}"#
        ))
        .unwrap();
        (src, file)
    }

    /// The one repo-grouped segment `split_segment_on_unmet_dependencies` splits, in
    /// every test below: three same-repo blocks, `A.1`, `A.2`, `A.3`.
    fn three_block_mev_segment() -> LaneSegment {
        let blocks = refs(&[
            ("A.1", "alpha", "mev"),
            ("A.2", "alpha", "mev"),
            ("A.3", "alpha", "mev"),
        ]);
        segment_lane_blocks(&blocks).into_iter().next().unwrap()
    }

    #[test]
    fn split_on_unmet_dependencies_operator_edge_splits_mid_run() {
        let seg = three_block_mev_segment();
        let loaded = vec![state_file_with_block(
            "mev",
            "A.2",
            "open",
            r#"[{"type":"operator","slug":"s","exit":"e","start":"c"}]"#,
        )];

        let out = split_segment_on_unmet_dependencies(seg, &loaded, &[]);
        assert_eq!(out.len(), 2, "expected a split at A.2, got {out:?}");
        assert_eq!(
            out[0].blocks.iter().map(|b| &b.id).collect::<Vec<_>>(),
            ["A.1"]
        );
        assert_eq!(
            out[1].blocks.iter().map(|b| &b.id).collect::<Vec<_>>(),
            ["A.2", "A.3"]
        );
    }

    #[test]
    fn split_on_unmet_dependencies_approval_edge_splits_mid_run() {
        let seg = three_block_mev_segment();
        let loaded = vec![state_file_with_block(
            "mev",
            "A.2",
            "open",
            r#"[{"type":"approval","slug":"s","what":"decide","digest":"d"}]"#,
        )];

        let out = split_segment_on_unmet_dependencies(seg, &loaded, &[]);
        assert_eq!(out.len(), 2, "expected a split at A.2, got {out:?}");
    }

    #[test]
    fn split_on_unmet_dependencies_external_edge_splits_mid_run() {
        let seg = three_block_mev_segment();
        let loaded = vec![state_file_with_block(
            "mev",
            "A.2",
            "open",
            r#"[{"type":"external","what":"waiting on a vendor"}]"#,
        )];

        let out = split_segment_on_unmet_dependencies(seg, &loaded, &[]);
        assert_eq!(out.len(), 2, "expected a split at A.2, got {out:?}");
    }

    #[test]
    fn split_on_unmet_dependencies_open_cross_repo_block_edge_splits() {
        let seg = three_block_mev_segment();
        let loaded = vec![
            state_file_with_block(
                "mev",
                "A.2",
                "open",
                r#"[{"type":"block","repo":"other","id":"X"}]"#,
            ),
            state_file_with_block("other", "X", "open", "[]"),
        ];

        let out = split_segment_on_unmet_dependencies(seg, &loaded, &[]);
        assert_eq!(
            out.len(),
            2,
            "expected a split on an open cross-repo target, got {out:?}"
        );
    }

    #[test]
    fn split_on_unmet_dependencies_closed_cross_repo_block_edge_does_not_split() {
        // Identical fixture to the test above, except the target is closed.
        let seg = three_block_mev_segment();
        let loaded = vec![
            state_file_with_block(
                "mev",
                "A.2",
                "open",
                r#"[{"type":"block","repo":"other","id":"X"}]"#,
            ),
            state_file_with_block("other", "X", "closed", "[]"),
        ];

        let out = split_segment_on_unmet_dependencies(seg, &loaded, &[]);
        assert_eq!(
            out.len(),
            1,
            "a closed target must never split, got {out:?}"
        );
        assert_eq!(out[0].blocks.len(), 3);
    }

    #[test]
    fn split_on_unmet_dependencies_same_repo_target_earlier_in_subsegment_does_not_split() {
        // A.2 depends on A.1 (open, same repo) — but A.1 already precedes A.2 in the
        // same sub-segment being built, so lane order already satisfies it.
        let seg = three_block_mev_segment();
        let loaded = vec![
            state_file_with_block(
                "mev",
                "A.2",
                "open",
                r#"[{"type":"block","repo":"mev","id":"A.1"}]"#,
            ),
            state_file_with_block("mev", "A.1", "open", "[]"),
        ];

        let out = split_segment_on_unmet_dependencies(seg, &loaded, &[]);
        assert_eq!(
            out.len(),
            1,
            "an order-satisfied same-segment dependency must not split, got {out:?}"
        );
        assert_eq!(out[0].blocks.len(), 3);
    }

    #[test]
    fn split_on_unmet_dependencies_open_target_absent_from_every_lane_file_splits() {
        // A.2 depends on mev:GHOST, open, same repo, not earlier in this sub-segment,
        // and absent from every lane file in the corpus — a real, invisible-to-any-lane
        // blocker.
        let seg = three_block_mev_segment();
        let loaded = vec![state_file_with_block(
            "mev",
            "A.2",
            "open",
            r#"[{"type":"block","repo":"mev","id":"GHOST"}]"#,
        )];

        let out = split_segment_on_unmet_dependencies(seg, &loaded, &[]);
        assert_eq!(
            out.len(),
            2,
            "an open target present in no lane file must split, got {out:?}"
        );
    }

    #[test]
    fn segment_lane_file_segments_renumbers_segment_and_position_after_a_split() {
        let lane_file = LaneFile {
            roadmap: "alpha".to_string(),
            lane: "substrate".to_string(),
            path: PathBuf::from("planning/roadmaps/alpha/lane-substrate.json"),
            blocks: refs(&[
                ("A.1", "alpha", "mev"),
                ("A.2", "alpha", "mev"),
                ("A.3", "alpha", "mev"),
            ]),
            directives: None,
        };
        let loaded = vec![state_file_with_block(
            "mev",
            "A.2",
            "open",
            r#"[{"type":"operator","slug":"s","exit":"e","start":"c"}]"#,
        )];

        let segments =
            segment_lane_file_segments(&lane_file, &loaded, std::slice::from_ref(&lane_file));

        assert_eq!(segments.len(), 2, "expected 2 segments, got {segments:?}");

        assert_eq!(segments[0].segment, 0);
        assert_eq!(segments[0].repo, "mev");
        assert_eq!(segments[0].blocks.len(), 1);
        assert_eq!(segments[0].blocks[0].id, "A.1");
        assert_eq!(segments[0].blocks[0].position, 0);

        assert_eq!(segments[1].segment, 1);
        assert_eq!(segments[1].repo, "mev");
        assert_eq!(segments[1].blocks.len(), 2);
        assert_eq!(segments[1].blocks[0].id, "A.2");
        assert_eq!(segments[1].blocks[0].position, 0);
        assert_eq!(segments[1].blocks[1].id, "A.3");
        assert_eq!(segments[1].blocks[1].position, 1);
    }

    // -- An authored repo the corpus does not know ------------------------------------

    fn owner_index_from(pairs: &[(&str, &[&str])]) -> OwnerIndex {
        let mut index: OwnerIndex = HashMap::new();
        for (id, repos) in pairs {
            index.insert(
                id.to_string(),
                repos.iter().map(|r| r.to_string()).collect(),
            );
        }
        index
    }

    #[test]
    fn unresolved_owner_diagnostics_fires_on_unknown_repo_and_does_not_abort() {
        let lane_file = LaneFile {
            roadmap: "close-the-loop".to_string(),
            lane: "substrate".to_string(),
            path: PathBuf::from("planning/close-the-loop/lane-substrate.json"),
            blocks: refs(&[
                ("A.1", "close-the-loop", "repoA"),
                ("GHOST.id", "close-the-loop", "repoGhost"),
                ("A.2", "close-the-loop", "repoA"),
            ]),
            directives: None,
        };
        let index = owner_index_from(&[("A.1", &["repoA"]), ("A.2", &["repoA"])]);

        let diags = unresolved_owner_diagnostics(&lane_file, &index);
        assert_eq!(
            diags.len(),
            1,
            "expected exactly one diagnostic, got {diags:?}"
        );
        assert_eq!(diags[0].severity, crate::Severity::Warning);
        assert_eq!(diags[0].file, lane_file.path);
        assert_eq!(diags[0].locator, "2"); // GHOST.id is the second block
        assert!(diags[0].message.contains("GHOST.id"));
        assert!(diags[0].message.contains("repoGhost"));
        assert!(diags[0].message.contains("does not know"));

        // Segmentation over the same data still proceeds — the block still renders
        // under its authored (unknown) repo, it is never omitted.
        let positions = segment_lane_file(&lane_file, &[], &[]);
        let ids: Vec<&str> = positions.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, vec!["A.1", "GHOST.id", "A.2"]);
    }

    #[test]
    fn unresolved_owner_diagnostics_empty_when_every_repo_is_known() {
        let lane_file = LaneFile {
            roadmap: "close-the-loop".to_string(),
            lane: "substrate".to_string(),
            path: PathBuf::from("planning/close-the-loop/lane-substrate.json"),
            blocks: refs(&[
                ("A.1", "close-the-loop", "repoA"),
                ("B.1", "close-the-loop", "repoB"),
            ]),
            directives: None,
        };
        let index = owner_index_from(&[("A.1", &["repoA"]), ("B.1", &["repoB"])]);

        let diags = unresolved_owner_diagnostics(&lane_file, &index);
        assert!(diags.is_empty(), "expected no diagnostics, got {diags:?}");
    }

    // -- derive_lane_positions: every appearance renders, unrepresentable double-claim -

    #[test]
    fn derive_lane_positions_ordinary_blocks_render_with_their_authored_origin_roadmap() {
        let files = vec![LaneFile {
            roadmap: "close-the-loop".to_string(),
            lane: "substrate".to_string(),
            path: PathBuf::from("lane-substrate.json"),
            blocks: refs(&[
                ("MV.ticket.a", "close-the-loop", "mev"),
                ("BT.ticket.b", "close-the-loop", "base-template"),
            ]),
            directives: None,
        }];
        let (positions, diags) = derive_lane_positions(&files, &[]);
        assert!(diags.is_empty());
        assert_eq!(positions.len(), 2);
        assert!(
            positions
                .iter()
                .all(|p| p.origin_roadmap == Some("close-the-loop".to_string()))
        );
    }

    #[test]
    fn derive_lane_positions_same_block_in_two_lane_files_renders_once_per_appearance() {
        // No cross-file double-claim exclusion any more: each lane record authors its
        // own origin_roadmap per block, so a block claimed in two lane files is not an
        // ambiguity — it renders under each executing lane, with its own authored
        // origin_roadmap.
        let files = vec![
            LaneFile {
                roadmap: "operator-surface".to_string(),
                lane: "substrate".to_string(),
                path: PathBuf::from("planning/operator-surface/lane-substrate.json"),
                blocks: refs(&[(
                    "OK.ticket.operator-edge-types",
                    "operator-in-the-loop",
                    "okf-core",
                )]),
                directives: None,
            },
            LaneFile {
                roadmap: "operator-in-the-loop".to_string(),
                lane: "brain".to_string(),
                path: PathBuf::from("planning/operator-in-the-loop/lane-brain.json"),
                blocks: refs(&[(
                    "OK.ticket.operator-edge-types",
                    "operator-in-the-loop",
                    "okf-core",
                )]),
                directives: None,
            },
        ];
        let (positions, diags) = derive_lane_positions(&files, &[]);
        assert!(diags.is_empty(), "expected no diagnostics, got {diags:?}");
        assert_eq!(
            positions.len(),
            2,
            "each appearance renders independently, got {positions:?}"
        );
        assert!(positions.iter().any(|p| p.roadmap == "operator-surface"));
        assert!(
            positions
                .iter()
                .any(|p| p.roadmap == "operator-in-the-loop")
        );
        assert!(
            positions
                .iter()
                .all(|p| p.origin_roadmap == Some("operator-in-the-loop".to_string()))
        );
    }

    // -----------------------------------------------------------------------
    // plan_lane_segments
    // -----------------------------------------------------------------------

    fn state_file_fixture(repo: &str, id: &str) -> (StateSource, StateFile) {
        let src = StateSource {
            repo_slug: repo.to_string(),
            abs_path: PathBuf::from(format!("{repo}/planning/state.json")),
            expected_kind: "project",
        };
        let file: StateFile = serde_json::from_str(&format!(
            r#"{{"repo":"{repo}","kind":"project","updated":"2026-08-14","tracks":[
                {{"title":"t","blocks":[{{"id":"{id}","title":"x","status":"open"}}]}}
            ]}}"#
        ))
        .unwrap();
        (src, file)
    }

    #[test]
    fn plan_lane_segments_writes_artifact_with_derived_positions() {
        let dir = crate::testsupport::unique_temp_dir("mev-plan-lane-segments-basic-json");
        write(
            &dir,
            "planning/roadmaps/alpha/lane-substrate.json",
            &simple_lane_json(
                "substrate",
                "alpha",
                &[
                    ("MV.ticket.a", "alpha", "mev"),
                    ("BT.ticket.b", "alpha", "base-template"),
                ],
            ),
        );
        let loaded = vec![
            state_file_fixture("mev", "MV.ticket.a"),
            state_file_fixture("base-template", "BT.ticket.b"),
        ];

        let plan = plan_lane_segments(&dir, &loaded);
        assert!(
            plan.diagnostics.is_empty(),
            "expected no diagnostics, got {:?}",
            plan.diagnostics
        );
        assert_eq!(plan.actions.len(), 1, "expected exactly one write action");

        let action = &plan.actions[0];
        assert_eq!(action.path, dir.join(LANE_SEGMENTS_ARTIFACT));

        let artifact: serde_json::Value =
            serde_json::from_str(&action.new_content).expect("artifact must be valid JSON");
        let blocks = artifact["blocks"].as_array().expect("blocks array");
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0]["roadmap"], "alpha");
        assert_eq!(blocks[0]["lane"], "substrate");
        assert_eq!(blocks[0]["repo"], "mev");
        assert_eq!(blocks[0]["segment"], 0);
        assert_eq!(blocks[0]["position"], 0);
        assert_eq!(blocks[0]["origin_roadmap"], "alpha");
        assert_eq!(blocks[1]["repo"], "base-template");
        assert_eq!(blocks[1]["segment"], 1);
        assert_eq!(blocks[1]["position"], 0);

        // A lane declaring no directives must serialise with NO "directives" key at
        // all — absence, not a null.
        for b in blocks {
            assert!(
                b.as_object().unwrap().get("directives").is_none(),
                "expected no 'directives' key for a lane declaring none, got {b}"
            );
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn plan_lane_segments_carries_directives_into_the_emitted_artifact() {
        let dir = crate::testsupport::unique_temp_dir("mev-plan-lane-segments-directives-json");
        write(
            &dir,
            "planning/roadmaps/alpha/lane-substrate.json",
            r#"{"lane":"substrate","roadmap":"alpha","blocks":[{"id":"MV.ticket.a","origin_roadmap":"alpha","repo":"mev"}],"held_until":"BA.19.C","budget":{"heavy":true,"not_with":["other-repo"]},"exclusive_repos":["mev","base-template"]}"#,
        );
        let loaded = vec![state_file_fixture("mev", "MV.ticket.a")];

        let plan = plan_lane_segments(&dir, &loaded);
        assert!(
            plan.diagnostics.is_empty(),
            "expected no diagnostics, got {:?}",
            plan.diagnostics
        );
        assert_eq!(plan.actions.len(), 1, "expected exactly one write action");

        let artifact: serde_json::Value = serde_json::from_str(&plan.actions[0].new_content)
            .expect("artifact must be valid JSON");
        let blocks = artifact["blocks"].as_array().expect("blocks array");
        assert_eq!(blocks.len(), 1);
        let directives = &blocks[0]["directives"];
        assert_eq!(directives["held_until"], "BA.19.C");
        assert_eq!(directives["budget"]["heavy"], true);
        assert_eq!(
            directives["budget"]["not_with"],
            serde_json::json!(["other-repo"])
        );
        assert_eq!(
            directives["exclusive_repos"],
            serde_json::json!(["mev", "base-template"])
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn plan_lane_segments_no_lane_files_plans_nothing() {
        let dir = crate::testsupport::unique_temp_dir("mev-plan-lane-segments-empty-json");
        std::fs::create_dir_all(dir.join("planning")).unwrap();

        let plan = plan_lane_segments(&dir, &[]);
        assert!(plan.actions.is_empty());
        assert!(plan.diagnostics.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn plan_lane_segments_unknown_repo_surfaces_a_diagnostic_but_still_writes() {
        let dir = crate::testsupport::unique_temp_dir("mev-plan-lane-segments-unknown-repo-json");
        write(
            &dir,
            "planning/roadmaps/alpha/lane-substrate.json",
            &simple_lane_json(
                "substrate",
                "alpha",
                &[("MV.ticket.unknown", "alpha", "ghost-repo")],
            ),
        );

        let plan = plan_lane_segments(&dir, &[]);
        assert!(
            plan.diagnostics
                .iter()
                .any(|d| d.message.contains("does not know")),
            "expected an unknown-repo diagnostic, got {:?}",
            plan.diagnostics
        );
        // The block still renders — an authored repo is never omitted, only flagged.
        assert_eq!(plan.actions.len(), 1);
        let artifact: serde_json::Value =
            serde_json::from_str(&plan.actions[0].new_content).unwrap();
        let blocks = artifact["blocks"].as_array().unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0]["repo"], "ghost-repo");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------------
    // roadmap_slug_from_plan_path / derive_program_membership
    // -----------------------------------------------------------------------

    #[test]
    fn roadmap_slug_from_plan_path_extracts_slug_from_a_roadmap_doc() {
        assert_eq!(
            roadmap_slug_from_plan_path("planning/roadmaps/lane-aware-briefing/roadmap.md"),
            Some("lane-aware-briefing".to_string())
        );
        assert_eq!(
            roadmap_slug_from_plan_path("planning/demand-ready/roadmap.md"),
            Some("demand-ready".to_string())
        );
    }

    #[test]
    fn roadmap_slug_from_plan_path_is_none_for_a_non_roadmap_doc() {
        // An area epic's plan doc — no roadmap slug to extract, and this must
        // not guess one from the filename.
        assert_eq!(
            roadmap_slug_from_plan_path("core/planning/epics/go-to-market.md"),
            None
        );
    }

    #[test]
    fn derive_program_membership_groups_by_executing_roadmap_in_derivation_order() {
        let dir = crate::testsupport::unique_temp_dir("mev-derive-program-membership-basic-json");
        write(
            &dir,
            "planning/roadmaps/alpha/lane-substrate.json",
            &simple_lane_json(
                "substrate",
                "alpha",
                &[
                    ("MV.ticket.a", "alpha", "mev"),
                    ("BT.ticket.b", "alpha", "base-template"),
                ],
            ),
        );

        let by_roadmap = derive_program_membership(&dir, &[]);
        let alpha = by_roadmap
            .get("alpha")
            .expect("alpha roadmap must have derived positions");
        let ids: Vec<&str> = alpha.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, vec!["MV.ticket.a", "BT.ticket.b"], "got {ids:?}");
        assert!(!by_roadmap.contains_key("bravo"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn derive_program_membership_is_empty_with_no_lane_files() {
        let dir = crate::testsupport::unique_temp_dir("mev-derive-program-membership-empty-json");
        std::fs::create_dir_all(dir.join("planning")).unwrap();

        let by_roadmap = derive_program_membership(&dir, &[]);
        assert!(by_roadmap.is_empty(), "got {by_roadmap:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn lane_directives_absent_is_distinguishable_from_empty_but_present() {
        // The whole safety property this ticket exists for: a lane declaring nothing
        // must never be mistaken for a lane that declared an empty constraint. `None`
        // vs. `Some(LaneDirectives::default())` are distinct values.
        let absent: Option<LaneDirectives> = None;
        let empty_but_present = Some(LaneDirectives::default());
        assert_ne!(
            absent, empty_but_present,
            "absence and an empty-but-present constraint must not compare equal"
        );

        let dir = crate::testsupport::unique_temp_dir("mev-lane-directives-absent-json");
        write(
            &dir,
            "planning/roadmaps/alpha/lane-substrate.json",
            &simple_lane_json("substrate", "alpha", &[("MV.ticket.a", "alpha", "mev")]),
        );
        let (files, diags) = discover_lane_files(&dir);
        assert!(diags.is_empty());
        assert_eq!(
            files[0].directives, absent,
            "a lane declaring nothing must produce None, never Some(default())"
        );
        assert_ne!(
            files[0].directives, empty_but_present,
            "a lane declaring nothing must not equal an empty-but-present constraint"
        );

        let none_repos = LaneDirectives {
            held_until: None,
            budget: None,
            exclusive_repos: None,
        };
        let empty_repos = LaneDirectives {
            held_until: None,
            budget: None,
            exclusive_repos: Some(Vec::new()),
        };
        assert_ne!(
            none_repos, empty_repos,
            "absent exclusive_repos must not equal an empty-but-present list"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------------
    // Frozen-contract golden tests — MV.17.A Task 3
    //
    // `core/engine-rs`'s `crates/engine-core/src/workflows/orchestration/chain.rs`
    // deserializes `planning/lane-segments.json` with `#[serde(deny_unknown_fields)]`
    // mirrors of `LaneDirectives`, `LaneBudget` and `DerivedBlockPosition`. A
    // round-trip test (serialize then deserialize back into the same Rust type)
    // would pass happily even after a field is added on both sides of that mirror —
    // it can only ever catch a mismatch this crate already knows about. Asserting
    // against a LITERAL expected JSON string is what actually fails the moment this
    // crate's serialized shape drifts from what engine-rs expects, whether or not
    // engine-rs's mirror was updated to match. Each pair below pins both the
    // fully-populated shape and the fully-absent shape, because `skip_serializing_if`
    // behaviour (a key omitted entirely, never emitted as `null`) is as much a part
    // of the frozen contract as the field names are.
    // -----------------------------------------------------------------------

    #[test]
    fn golden_lane_directives_all_fields_present() {
        let directives = LaneDirectives {
            held_until: Some("2026-09-01".to_string()),
            budget: Some(LaneBudget {
                heavy: true,
                not_with: vec!["other-repo".to_string()],
            }),
            exclusive_repos: Some(vec!["mev".to_string(), "base-template".to_string()]),
        };
        let got = serde_json::to_string(&directives).unwrap();
        let expected = r#"{"held_until":"2026-09-01","budget":{"heavy":true,"not_with":["other-repo"]},"exclusive_repos":["mev","base-template"]}"#;
        assert_eq!(
            got, expected,
            "LaneDirectives serialized shape drifted from the frozen contract engine-rs's chain.rs mirrors"
        );
    }

    #[test]
    fn golden_lane_directives_all_fields_absent() {
        let directives = LaneDirectives {
            held_until: None,
            budget: None,
            exclusive_repos: None,
        };
        let got = serde_json::to_string(&directives).unwrap();
        assert_eq!(
            got, "{}",
            "an all-absent LaneDirectives must serialize to an empty object — no key present as null"
        );
    }

    #[test]
    fn golden_lane_budget_all_fields_present() {
        let budget = LaneBudget {
            heavy: false,
            not_with: vec!["repo-a".to_string(), "repo-b".to_string()],
        };
        let got = serde_json::to_string(&budget).unwrap();
        let expected = r#"{"heavy":false,"not_with":["repo-a","repo-b"]}"#;
        assert_eq!(
            got, expected,
            "LaneBudget serialized shape drifted from the frozen contract engine-rs's chain.rs mirrors"
        );
    }

    #[test]
    fn golden_lane_budget_not_with_absent() {
        let budget = LaneBudget {
            heavy: true,
            not_with: Vec::new(),
        };
        let got = serde_json::to_string(&budget).unwrap();
        assert_eq!(
            got, r#"{"heavy":true}"#,
            "an empty not_with must be omitted entirely, not emitted as an empty array"
        );
    }

    #[test]
    fn golden_derived_block_position_all_fields_present() {
        let pos = DerivedBlockPosition {
            roadmap: "alpha".to_string(),
            lane: "substrate".to_string(),
            repo: "mev".to_string(),
            id: "MV.ticket.a".to_string(),
            line: 12,
            segment: 0,
            position: 1,
            origin_roadmap: Some("alpha".to_string()),
            directives: Some(LaneDirectives {
                held_until: Some("2026-09-01".to_string()),
                budget: Some(LaneBudget {
                    heavy: true,
                    not_with: vec!["other-repo".to_string()],
                }),
                exclusive_repos: Some(vec!["mev".to_string()]),
            }),
        };
        let got = serde_json::to_string(&pos).unwrap();
        let expected = r#"{"roadmap":"alpha","lane":"substrate","repo":"mev","id":"MV.ticket.a","line":12,"segment":0,"position":1,"origin_roadmap":"alpha","directives":{"held_until":"2026-09-01","budget":{"heavy":true,"not_with":["other-repo"]},"exclusive_repos":["mev"]}}"#;
        assert_eq!(
            got, expected,
            "DerivedBlockPosition serialized shape drifted from the frozen contract engine-rs's chain.rs mirrors"
        );
    }

    #[test]
    fn golden_derived_block_position_all_fields_absent() {
        let pos = DerivedBlockPosition {
            roadmap: "alpha".to_string(),
            lane: "substrate".to_string(),
            repo: "mev".to_string(),
            id: "MV.ticket.a".to_string(),
            line: 12,
            segment: 0,
            position: 1,
            origin_roadmap: None,
            directives: None,
        };
        let got = serde_json::to_string(&pos).unwrap();
        let expected = r#"{"roadmap":"alpha","lane":"substrate","repo":"mev","id":"MV.ticket.a","line":12,"segment":0,"position":1,"origin_roadmap":null}"#;
        assert_eq!(
            got, expected,
            "origin_roadmap has no skip_serializing_if — it must serialize as null when absent; \
             directives DOES have skip_serializing_if — it must be omitted entirely, not emitted as null"
        );
    }
}
