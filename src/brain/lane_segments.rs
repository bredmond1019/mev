//! Lane file discovery and parsing — `MV.13.A` Task 1.
//!
//! A "lane file" (`lane-*.txt`) is the authored, human-readable execution order for one
//! roadmap's slice of work: an ordered list of block IDs, one per line, with `#` comments
//! carrying binding context for the operator running it (`/begin-orchestration` Step 1F).
//! No service reads that structure today — this module is the first thing that turns a
//! lane file into a derived object another consumer (Task 2's segmentation, `emit-state`)
//! can act on.
//!
//! # Comments are opaque, with one declared exception
//!
//! `#` comments are stripped when extracting the block-ID list and are never parsed for
//! structure — they are prose for the human, not directives for this walker. The single
//! exception is `# ORIGIN:` (MV.13.A Task 4, below): a fixed-prefix, declared
//! machine-readable directive, not prose, and it attaches to exactly the one block-ID
//! line that immediately follows it — never a run of blocks, however the human-facing
//! text beside it reads. Any other comment, including one that reads like repo-boundary
//! prose, changes nothing; see `parse_lane_blocks_repo_boundary_prose_in_comment_is_inert`
//! below for the pinning test.
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
//! encoded — nothing in `state.json` mirrors it. The `lane-*.txt` glob must never widen to
//! catch it.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::Diagnostic;
use crate::brain::state::{StateFile, StateSource};

/// Name of the directory holding roadmaps under the current (non-legacy) layout.
const ROADMAPS_DIR: &str = "roadmaps";

/// Directory names that are never roadmap slugs, even though they sit directly under
/// `planning/` alongside legacy roadmap directories.
const NON_ROADMAP_DIR_NAMES: &[&str] = &["archive", "decisions", "artifacts"];

/// One block-ID reference inside a lane file: the raw ID text plus the 1-based line it
/// came from, so a downstream diagnostic (Task 3's unresolvable-ID warning) can name a
/// precise location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneBlockRef {
    pub id: String,
    pub line: usize,
    /// The `# ORIGIN:` directive (Task 4, below) attached to this exact block-ID line,
    /// if the immediately preceding comment line declared one. `None` for the ordinary
    /// case — no annotation, not even an implicit "no origin".
    pub origin: Option<Origin>,
}

/// One discovered, parsed `lane-*.txt` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneFile {
    /// The owning roadmap's slug — the name of the directory directly containing this
    /// lane file, whether that directory sits under `planning/roadmaps/` or legacy
    /// `planning/<slug>/`.
    pub roadmap: String,
    /// The lane name: the filename with the `lane-` prefix and `.txt` suffix stripped
    /// (`lane-substrate.txt` → `substrate`).
    pub lane: String,
    /// Absolute path to the file on disk.
    pub path: PathBuf,
    /// The ordered, comment-and-blank-stripped block-ID list. File order is execution
    /// order and is preserved exactly — never sorted, deduped, or normalised.
    pub blocks: Vec<LaneBlockRef>,
}

/// Discover every live `lane-*.txt` file under `root/planning/roadmaps/<slug>/` and legacy
/// `root/planning/<slug>/`, excluding anything under an `archive/` directory at any depth
/// and any file or directory whose name starts with `_` (the corpus-wide ephemeral/debug
/// convention). Symlinks are followed — every `planning/` in this fleet is itself a
/// symlink into a `_planning/` vault, and a symlink-blind walk would silently return a
/// subset while looking successful.
///
/// Returns the discovered, parsed lane files plus diagnostics for structural problems.
/// Today the only such diagnostic is a roadmap slug claimed by both layouts at once.
pub fn discover_lane_files(root: &Path) -> (Vec<LaneFile>, Vec<Diagnostic>) {
    let planning_dir = root.join("planning");
    let mut lane_files = Vec::new();
    let mut diags = Vec::new();

    if !planning_dir.is_dir() {
        return (lane_files, diags);
    }

    let mut roadmaps_slugs: HashSet<String> = HashSet::new();
    let mut legacy_slugs: HashSet<String> = HashSet::new();

    // Current layout: planning/roadmaps/<slug>/lane-*.txt
    let roadmaps_dir = planning_dir.join(ROADMAPS_DIR);
    if roadmaps_dir.is_dir() {
        for (slug, slug_dir) in child_dirs(&roadmaps_dir) {
            roadmaps_slugs.insert(slug.clone());
            collect_lane_files_in(&slug_dir, &slug, &mut lane_files, &mut diags);
        }
    }

    // Legacy layout: planning/<slug>/lane-*.txt — every direct child of planning/ except
    // roadmaps/ itself and the non-roadmap directories that live alongside roadmap dirs.
    for (slug, slug_dir) in child_dirs(&planning_dir) {
        if slug == ROADMAPS_DIR || NON_ROADMAP_DIR_NAMES.contains(&slug.as_str()) {
            continue;
        }
        // Only a directory that actually contains a lane file counts as a "legacy
        // roadmap directory" for the both-locations check below — many planning/
        // children (spec dirs, orchestration-run, etc.) are not roadmaps at all and
        // must not collide with a same-named roadmaps/ entry that has none either.
        let found_before = lane_files.len();
        collect_lane_files_in(&slug_dir, &slug, &mut lane_files, &mut diags);
        if lane_files.len() > found_before {
            legacy_slugs.insert(slug);
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

/// Walk `slug_dir` (following symlinks) for `lane-*.txt` files, excluding anything under
/// an `archive` path segment and any `_`-prefixed file, parse each, and push a
/// [`LaneFile`] per hit (plus a diagnostic on unreadable content).
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
                    format!("could not read lane file: {e}"),
                ));
                continue;
            }
        };

        lane_files.push(LaneFile {
            roadmap: slug.to_string(),
            lane,
            path,
            blocks: parse_lane_blocks(&content),
        });
    }
}

/// `true` iff `file_name` matches the `lane-*.txt` glob and is not excluded by the
/// leading-`_` debug-file convention. `deferred-blocks.txt` deliberately does not match
/// this (no `lane-` prefix) — it is out of scope for this module by design.
fn is_lane_file_name(file_name: &str) -> bool {
    !file_name.starts_with('_')
        && file_name.starts_with("lane-")
        && file_name.ends_with(".txt")
        && file_name.len() > "lane-".len() + ".txt".len()
}

/// `lane-substrate.txt` → `substrate`.
fn lane_name_from_file(file_name: &str) -> String {
    file_name
        .strip_prefix("lane-")
        .and_then(|s| s.strip_suffix(".txt"))
        .unwrap_or(file_name)
        .to_string()
}

/// Parse a lane file's raw content into an ordered block-ID list: `#` comments and blank
/// lines are stripped, everything else is kept in file order with its 1-based line
/// number. Never sorts, dedupes, or normalises — file order is execution order.
///
/// The one exception to "comments are stripped and ignored": a comment-only line whose
/// body (after `#` and leading whitespace) starts with the fixed prefix `ORIGIN:` is
/// parsed as an [`Origin`] directive (Task 4) and attached to the single next block-ID
/// line. Any other comment — including further prose continuing the same `# ORIGIN:`
/// annotation, or text that merely *reads* like a directive — is inert.
pub fn parse_lane_blocks(content: &str) -> Vec<LaneBlockRef> {
    let mut out = Vec::new();
    let mut pending_origin: Option<Origin> = None;
    for (idx, raw_line) in content.lines().enumerate() {
        let line = idx + 1;
        let before_comment = match raw_line.find('#') {
            Some(pos) => &raw_line[..pos],
            None => raw_line,
        };
        let id = before_comment.trim();
        if id.is_empty() {
            // Comment-only or blank line. The one thing worth looking at here is the
            // `# ORIGIN:` directive; everything else is prose and is dropped.
            if let Some(pos) = raw_line.find('#') {
                let comment_body = raw_line[pos + 1..].trim_start();
                if let Some(value) = comment_body.strip_prefix("ORIGIN:") {
                    pending_origin = Some(Origin::parse(value.trim()));
                }
            }
            continue;
        }
        out.push(LaneBlockRef {
            id: id.to_string(),
            line,
            origin: pending_origin.take(),
        });
    }
    out
}

// ---------------------------------------------------------------------------
// Ownership resolution and segmentation — MV.13.A Task 2
// ---------------------------------------------------------------------------

/// Corpus-wide index from a literal block-ID string to every repo slug that owns a
/// block by that exact ID, built from `state.json`'s `tracks[].blocks[]` across every
/// loaded file. State-graph keys are `repo:id` (see [`crate::brain::block_graph`]), so
/// this index groups by repo *before* collapsing to a single owner — a bare ID that
/// happens to be authored under two repos is a legitimate corpus state, not a bug, and
/// [`resolve_owner`] refuses to guess between them.
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

/// Resolve a bare lane-file block ID to its single owning repo slug via `index`.
///
/// Returns `None` — never a guess — when the ID resolves to **zero** repos (unknown to
/// the corpus; Task 3 turns this into a diagnostic) or to **more than one** (a bare ID
/// legitimately reused across repos; resolving that ambiguity would require the actual
/// `repo:id` graph key, which a lane file's bare ID does not carry). Nearest-neighbour,
/// "the file's other blocks", or directory-based guessing are explicitly ruled out by
/// the spec — this function embodies that by returning `None` rather than picking one.
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
    /// 1-based source line, carried through from [`LaneBlockRef`].
    pub line: usize,
    /// 0-based index of this block within its segment — the second half of the
    /// `{segment, position}` pair. Chosen zero-based to match [`LaneSegment::segment`]
    /// and every other zero-based index already in this derivation (`topo_index`,
    /// vec indices generally); documented once here, not re-chosen per call site.
    pub position: usize,
}

/// One contiguous run of a lane's blocks all owned by the same repo — the `(repo,
/// chain)` segment this task derives. **Derived at emit time; never authored** — a lane
/// file stays the authoring surface and nothing here is written back into one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneSegment {
    pub repo: String,
    /// 0-based index of this segment within its lane.
    pub segment: usize,
    /// Ordered, in file order — never sorted, deduped, or normalised.
    pub blocks: Vec<LaneSegmentBlock>,
}

impl LaneSegment {
    /// The segment's first block — the block `MV.13.B`'s (deferred) frontier
    /// computation will read. Exposed cleanly now rather than foreclosed later;
    /// `MV.13.A` itself does nothing with it beyond this accessor.
    pub fn head(&self) -> Option<&LaneSegmentBlock> {
        self.blocks.first()
    }
}

/// Segment an ordered block-ID list into `(repo, chain)` runs by cutting a new segment
/// at every ownership change, via `resolve` (typically [`resolve_owner`] over a
/// corpus-wide [`OwnerIndex`]).
///
/// Order is preserved exactly, within and across segments — a repo appearing twice
/// **non-contiguously** (e.g. `A, B, A`) yields **three** segments, never one merged
/// segment; that non-contiguous case is what separates this from a plain `group_by`.
///
/// A block whose owner does not resolve (`resolve` returns `None`) is **omitted** from
/// every segment — it contributes no cut and appears in no segment's `blocks`. Task 3
/// turns each such omission into a diagnostic; this function only segments what it can
/// attribute, and never guesses to keep a block in.
pub fn segment_lane_blocks(
    blocks: &[LaneBlockRef],
    mut resolve: impl FnMut(&str) -> Option<String>,
) -> Vec<LaneSegment> {
    let mut segments: Vec<LaneSegment> = Vec::new();
    for block in blocks {
        let Some(repo) = resolve(&block.id) else {
            continue;
        };
        let starts_new_segment = match segments.last() {
            Some(seg) => seg.repo != repo,
            None => true,
        };
        if starts_new_segment {
            segments.push(LaneSegment {
                repo,
                segment: segments.len(),
                blocks: Vec::new(),
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
        });
    }
    segments
}

/// One block, positioned within its lane after ownership-based segmentation —
/// `{roadmap, lane, segment, position}`, the per-block derived record this task
/// produces. Derived at emit time; authored nowhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneBlockPosition {
    pub roadmap: String,
    pub lane: String,
    pub repo: String,
    pub id: String,
    pub line: usize,
    pub segment: usize,
    pub position: usize,
}

/// Segment one [`LaneFile`]'s blocks (via [`segment_lane_blocks`]) and flatten the
/// result into one [`LaneBlockPosition`] per resolvable block, each carrying its
/// owning lane file's `roadmap`/`lane`.
pub fn segment_lane_file(
    lane_file: &LaneFile,
    resolve: impl FnMut(&str) -> Option<String>,
) -> Vec<LaneBlockPosition> {
    segment_lane_blocks(&lane_file.blocks, resolve)
        .into_iter()
        .flat_map(|seg| {
            let LaneSegment {
                repo,
                segment,
                blocks,
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
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Unresolvable IDs are a diagnostic, never a guess — MV.13.A Task 3
// ---------------------------------------------------------------------------

/// Emit a diagnostic for every block in `lane_file` whose owner does not resolve via
/// `index`, naming the file, the 1-based line, and the ID.
///
/// Two distinct resolution failures both count here, and both get a diagnostic rather
/// than a silent omission from every segment:
/// - **unknown** — the ID matches zero repos in the corpus. A lane file may legitimately
///   reference a block filed later than the lane file itself, so this is expected to
///   happen in the ordinary course of authoring, not just on a typo.
/// - **ambiguous** — the bare ID matches more than one repo (graph keys are `repo:id`;
///   see [`build_owner_index`]). [`resolve_owner`] refuses to pick one, so this is
///   surfaced too rather than resolved first-wins.
///
/// **Warning, never error, and never aborts derivation.** A hard error here would
/// red-gate the whole corpus for every concurrent lane over an authoring-order detail —
/// segmentation ([`segment_lane_blocks`]) already proceeds over whatever *did* resolve,
/// omitting the rest; this function only reports what got left out and why. Never
/// guesses an owner — not by nearest neighbour, not by the file's other blocks, not by
/// the containing directory.
pub fn unresolved_owner_diagnostics(lane_file: &LaneFile, index: &OwnerIndex) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for block in &lane_file.blocks {
        let repos: &[String] = index.get(&block.id).map(|v| v.as_slice()).unwrap_or(&[]);
        let reason = match repos.len() {
            0 => {
                Some("unknown to the corpus — no repo's state.json owns this block ID".to_string())
            }
            1 => None,
            n => Some(format!(
                "ambiguous — owned by {n} repos ({}), a bare ID cannot resolve to a single owner",
                repos.join(", ")
            )),
        };
        let Some(reason) = reason else { continue };
        diags.push(Diagnostic::warning(
            &lane_file.path,
            block.line.to_string(),
            format!(
                "lane block '{}' could not resolve an owner: {reason}",
                block.id
            ),
        ));
    }
    diags
}

// ---------------------------------------------------------------------------
// Double-claim validation and `# ORIGIN:` — MV.13.A Task 4
// ---------------------------------------------------------------------------

/// The `# ORIGIN:` directive's parsed value — the one comment this module treats as
/// structure rather than prose. Honoured per base-template D57's two-axis rule: the
/// block's `roadmap` (Task 2's derivation) is always the *executing* lane — the lane
/// file the block-ID line physically lives in — and `origin_roadmap` (below) is the
/// *owning* roadmap this annotation names. The two are never the same field, and a
/// block is never rendered under both.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// `# ORIGIN: none` — declared explicitly: this block is not adopted from another
    /// roadmap (e.g. a standalone chore folded into a lane for scheduling reasons).
    None,
    /// `# ORIGIN: <path-or-slug>` — the owning roadmap. The live corpus writes this as
    /// an absolute path to that roadmap's `roadmap.md`; the slug is the path's parent
    /// directory name. A bare slug (no `roadmap.md` suffix) is accepted as-is.
    Roadmap(String),
}

impl Origin {
    /// Parse the text after `# ORIGIN:` (already trimmed of the prefix). Only the first
    /// whitespace-delimited token is machine data — the live corpus continues the
    /// annotation onto further comment lines of human-facing prose (e.g. "the two
    /// adopted blocks below — their run-record ledger rows carry..."), which this
    /// module never reads.
    fn parse(value: &str) -> Self {
        let token = value.split_whitespace().next().unwrap_or(value);
        if token.eq_ignore_ascii_case("none") {
            Origin::None
        } else {
            Origin::Roadmap(roadmap_slug_from_origin_token(token))
        }
    }
}

/// Extract a roadmap slug from an `# ORIGIN:` token. `/path/to/planning/<slug>/roadmap.md`
/// yields `<slug>`; anything else (already a bare slug, or a differently-shaped path) is
/// returned trimmed of a trailing `/` and otherwise unchanged — this module does not
/// require the corpus's one live shape to be the only one it can parse.
fn roadmap_slug_from_origin_token(token: &str) -> String {
    let trimmed = token.trim_end_matches('/');
    let path = Path::new(trimmed);
    if path.file_name().is_some_and(|f| f == "roadmap.md")
        && let Some(slug) = path.parent().and_then(|p| p.file_name())
    {
        return slug.to_string_lossy().to_string();
    }
    trimmed.to_string()
}

/// One block ID's double-claim resolution: which lane file's claim renders, and the
/// `origin_roadmap` it carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimResolution {
    pub id: String,
    /// The executing claim's roadmap — matches the [`LaneFile::roadmap`] of exactly one
    /// of the claiming files.
    pub roadmap: String,
    pub lane: String,
    pub line: usize,
    /// The owning roadmap named by the executing claim's `# ORIGIN:` annotation.
    /// `None` when the annotation was `# ORIGIN: none`.
    pub origin_roadmap: Option<String>,
}

/// Resolve every block ID claimed by more than one **distinct roadmap** across
/// `lane_files`, honouring `# ORIGIN:` per D57's two-axis rule.
///
/// An ID claimed by only one roadmap — however many lane files or lines within that one
/// roadmap repeat it — is not a double-claim and produces neither a resolution nor a
/// diagnostic; this function only looks at cross-roadmap claims.
///
/// - **Unannotated double-claim** (no claiming instance of this ID carries `# ORIGIN:`):
///   an `E_LANE_DOUBLE_CLAIM` error diagnostic naming every claiming file, line and
///   roadmap, and **no** [`ClaimResolution`]. Never resolved first-wins — the roadmap's
///   Q5 rule renders a block under exactly one executing roadmap, and guessing between
///   two would silently misstate both lanes' remaining depth.
/// - **Ambiguous double-claim** (more than one claiming instance carries `# ORIGIN:`):
///   also an `E_LANE_DOUBLE_CLAIM` error — two "this is where it's adopted"
///   declarations for one block is the same ambiguity, only authored instead of
///   implicit, so it gets the same treatment: no resolution, never first-wins.
/// - **Resolved double-claim** (exactly one claiming instance carries `# ORIGIN:`): one
///   [`ClaimResolution`] naming that instance as executing, and no diagnostic.
pub fn resolve_double_claims(lane_files: &[LaneFile]) -> (Vec<ClaimResolution>, Vec<Diagnostic>) {
    let mut claims: HashMap<&str, Vec<(usize, &LaneBlockRef)>> = HashMap::new();
    for (fi, lf) in lane_files.iter().enumerate() {
        for b in &lf.blocks {
            claims.entry(b.id.as_str()).or_default().push((fi, b));
        }
    }

    let mut ids: Vec<&str> = claims.keys().copied().collect();
    ids.sort_unstable(); // deterministic diagnostic/resolution order

    let mut resolutions = Vec::new();
    let mut diags = Vec::new();

    for id in ids {
        let refs = &claims[id];
        let mut distinct_roadmaps: Vec<&str> = Vec::new();
        for (fi, _) in refs {
            let r = lane_files[*fi].roadmap.as_str();
            if !distinct_roadmaps.contains(&r) {
                distinct_roadmaps.push(r);
            }
        }
        if distinct_roadmaps.len() <= 1 {
            continue; // one roadmap owns every claim — not a double-claim.
        }

        let annotated: Vec<&(usize, &LaneBlockRef)> =
            refs.iter().filter(|(_, b)| b.origin.is_some()).collect();

        let describe = |claimants: &[&(usize, &LaneBlockRef)]| -> String {
            claimants
                .iter()
                .map(|(fi, b)| {
                    format!(
                        "{} (roadmap '{}', line {})",
                        lane_files[*fi].path.display(),
                        lane_files[*fi].roadmap,
                        b.line
                    )
                })
                .collect::<Vec<_>>()
                .join("; ")
        };

        match annotated.len() {
            0 => {
                let all: Vec<&(usize, &LaneBlockRef)> = refs.iter().collect();
                diags.push(Diagnostic::error(
                    &lane_files[refs[0].0].path,
                    "E_LANE_DOUBLE_CLAIM",
                    format!(
                        "block '{id}' is claimed by {} different roadmaps with no '# ORIGIN:' \
                         annotation resolving the ambiguity — never resolved first-wins: {}",
                        distinct_roadmaps.len(),
                        describe(&all)
                    ),
                ));
            }
            1 => {
                let (fi, b) = *annotated[0];
                let origin_roadmap = match b.origin.as_ref().expect("filtered on is_some") {
                    Origin::None => None,
                    Origin::Roadmap(slug) => Some(slug.clone()),
                };
                resolutions.push(ClaimResolution {
                    id: id.to_string(),
                    roadmap: lane_files[fi].roadmap.clone(),
                    lane: lane_files[fi].lane.clone(),
                    line: b.line,
                    origin_roadmap,
                });
            }
            _ => {
                diags.push(Diagnostic::error(
                    &lane_files[annotated[0].0].path,
                    "E_LANE_DOUBLE_CLAIM",
                    format!(
                        "block '{id}' carries '# ORIGIN:' in more than one claiming lane file \
                         — ambiguous which is executing, never resolved first-wins: {}",
                        describe(&annotated)
                    ),
                ));
            }
        }
    }

    (resolutions, diags)
}

/// One block's final derived position, corpus-wide: Task 2's `{roadmap, lane, segment,
/// position}` plus `origin_roadmap` (`None` unless this block was a resolved
/// double-claim).
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
}

/// Derive `{roadmap, lane, segment, position}` for every block across `lane_files`,
/// resolving double-claims (via [`resolve_double_claims`]) before segmenting each file.
///
/// A block ID that is an **unresolved** double-claim (unannotated or ambiguously
/// annotated) is excluded from every claiming file's derivation entirely — Task 3's
/// "never guess" rule extended to double-claims, since the diagnostic already explains
/// why and silently picking one anyway would defeat it. A **resolved** double-claim
/// renders only from its executing `(roadmap, lane)`; every other claiming file skips
/// it, so it is never rendered — and never counted — twice.
pub fn derive_lane_positions(
    lane_files: &[LaneFile],
    mut resolve_owner: impl FnMut(&str) -> Option<String>,
) -> (Vec<DerivedBlockPosition>, Vec<Diagnostic>) {
    let (resolutions, diags) = resolve_double_claims(lane_files);

    // Distinct roadmaps per id across the whole corpus — matches resolve_double_claims'
    // own computation, so a block excluded here is exactly a block that function saw as
    // a double-claim.
    let mut claim_roadmaps: HashMap<&str, Vec<&str>> = HashMap::new();
    for lf in lane_files {
        for b in &lf.blocks {
            let v = claim_roadmaps.entry(b.id.as_str()).or_default();
            if !v.contains(&lf.roadmap.as_str()) {
                v.push(lf.roadmap.as_str());
            }
        }
    }

    let resolved_by_id: HashMap<&str, &ClaimResolution> =
        resolutions.iter().map(|r| (r.id.as_str(), r)).collect();

    let mut out = Vec::new();
    for lf in lane_files {
        let allowed: Vec<LaneBlockRef> = lf
            .blocks
            .iter()
            .filter(|b| {
                let distinct_roadmaps =
                    claim_roadmaps.get(b.id.as_str()).map(Vec::len).unwrap_or(1);
                if distinct_roadmaps <= 1 {
                    return true; // not a double-claim at all
                }
                match resolved_by_id.get(b.id.as_str()) {
                    Some(res) => res.roadmap == lf.roadmap && res.lane == lf.lane,
                    None => false, // unresolved double-claim: excluded everywhere
                }
            })
            .cloned()
            .collect();

        let filtered = LaneFile {
            roadmap: lf.roadmap.clone(),
            lane: lf.lane.clone(),
            path: lf.path.clone(),
            blocks: allowed,
        };
        for p in segment_lane_file(&filtered, &mut resolve_owner) {
            let origin_roadmap = resolved_by_id
                .get(p.id.as_str())
                .and_then(|r| r.origin_roadmap.clone());
            out.push(DerivedBlockPosition {
                roadmap: p.roadmap,
                lane: p.lane,
                repo: p.repo,
                id: p.id,
                line: p.line,
                segment: p.segment,
                position: p.position,
                origin_roadmap,
            });
        }
    }

    (out, diags)
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
#[derive(Debug, Clone, serde::Serialize)]
struct LaneSegmentsArtifact {
    /// Every lane file's derived block positions, corpus-wide, in the same order
    /// [`derive_lane_positions`] returns them — lane-file discovery order, then file
    /// order within each lane. Never re-sorted here.
    blocks: Vec<DerivedBlockPosition>,
}

/// Plan the [`LANE_SEGMENTS_ARTIFACT`] write: discover every live lane file under
/// `root`, resolve ownership from `loaded`'s corpus-wide `state.json` graph keys, and
/// derive `{roadmap, lane, segment, position}` (plus double-claim resolution) for
/// every block that resolves — Tasks 1 through 4, assembled into one [`EmitPlan`] for
/// `emit_state` to apply alongside its other planners.
///
/// Diagnostics carried on the returned plan are the union of:
/// - [`discover_lane_files`]'s structural diagnostics (e.g. a slug claimed by both
///   roadmap layouts at once);
/// - [`unresolved_owner_diagnostics`] for every block in every lane file, run before
///   double-claim exclusion so an ID that is both unresolvable *and* would have been a
///   double-claim still gets the "unknown to the corpus" warning it deserves;
/// - [`derive_lane_positions`]'s own `E_LANE_DOUBLE_CLAIM` errors.
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

    let (blocks, double_claim_diags) = derive_lane_positions(&lane_files, |id| {
        resolve_owner(&owner_index, id).map(str::to_string)
    });
    plan.diagnostics.extend(double_claim_diags);

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

/// Derive every lane-file block's position, corpus-wide, grouped by its **executing**
/// roadmap slug — the feed [`crate::brain::emit::epic_members_resolved`] (`MV.13.D`
/// Task 3) consumes for `kind: program` epics.
///
/// Each roadmap's `Vec` preserves [`derive_lane_positions`]'s own output order exactly
/// (lane-file discovery order, then each file's segment/position order) — `segment` and
/// `position` are indices local to one lane file, not a corpus-wide sort key, so
/// re-sorting by them across lane files would silently interleave unrelated files'
/// blocks. A block that is a resolved double-claim (`# ORIGIN:`) already appears only
/// under its executing roadmap here, because [`derive_lane_positions`] excludes it from
/// every other claiming file — the same `origin_roadmap` guarantee this function's
/// caller relies on to never render a block under two programs.
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

    let owner_index = build_owner_index(files);
    let (blocks, _double_claim_diags) = derive_lane_positions(&lane_files, |id| {
        resolve_owner(&owner_index, id).map(str::to_string)
    });

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

    #[test]
    fn parse_lane_blocks_strips_comments_and_blanks_preserving_order() {
        let content = "\
# Lane SUBSTRATE — binding context for the operator
# ROADMAP: /path/to/roadmap.md
\n\
MV.ticket.first
# a comment-only line
BT.ticket.second

OK.ticket.third
";
        let blocks = parse_lane_blocks(content);
        let ids: Vec<&str> = blocks.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["MV.ticket.first", "BT.ticket.second", "OK.ticket.third"]
        );
        // Line numbers point at the real source line, not a post-strip index.
        assert_eq!(blocks[0].line, 4);
        assert_eq!(blocks[1].line, 6);
        assert_eq!(blocks[2].line, 8);
    }

    #[test]
    fn parse_lane_blocks_never_sorts_or_dedupes() {
        // A repo appearing twice non-contiguously must survive as two entries in order —
        // segmentation (Task 2) depends on this being preserved, not deduped here.
        let content = "B.one\nA.one\nB.two\n";
        let blocks = parse_lane_blocks(content);
        let ids: Vec<&str> = blocks.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(ids, vec!["B.one", "A.one", "B.two"]);
    }

    #[test]
    fn discover_lane_files_finds_both_layouts() {
        let dir = crate::testsupport::unique_temp_dir("mev-lane-discover-both-layouts");
        write(
            &dir,
            "planning/roadmaps/alpha/lane-substrate.txt",
            "MV.ticket.a\nBT.ticket.b\n",
        );
        write(&dir, "planning/beta/lane-gtm.txt", "OK.ticket.c\n");

        let (files, diags) = discover_lane_files(&dir);
        assert!(diags.is_empty(), "expected no diagnostics, got {diags:?}");
        assert_eq!(files.len(), 2, "expected two lane files, got {files:?}");

        let alpha = files.iter().find(|f| f.roadmap == "alpha").unwrap();
        assert_eq!(alpha.lane, "substrate");
        assert_eq!(alpha.blocks.len(), 2);

        let beta = files.iter().find(|f| f.roadmap == "beta").unwrap();
        assert_eq!(beta.lane, "gtm");
        assert_eq!(beta.blocks.len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn discover_lane_files_excludes_archive_and_underscore_prefixed() {
        let dir = crate::testsupport::unique_temp_dir("mev-lane-discover-exclusions");
        write(
            &dir,
            "planning/roadmaps/alpha/archive/lane-old.txt",
            "MV.ticket.old\n",
        );
        write(
            &dir,
            "planning/roadmaps/alpha/_scratch/lane-debug.txt",
            "MV.ticket.debug\n",
        );
        write(
            &dir,
            "planning/roadmaps/alpha/_lane-underscore.txt",
            "MV.ticket.underscore\n",
        );
        write(
            &dir,
            "planning/roadmaps/alpha/lane-live.txt",
            "MV.ticket.live\n",
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
        let dir = crate::testsupport::unique_temp_dir("mev-lane-discover-deferred");
        write(
            &dir,
            "planning/roadmaps/alpha/deferred-blocks.txt",
            "MV.ticket.deferred\n",
        );
        write(
            &dir,
            "planning/roadmaps/alpha/lane-live.txt",
            "MV.ticket.live\n",
        );

        let (files, _diags) = discover_lane_files(&dir);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].lane, "live");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn discover_lane_files_both_layouts_same_slug_is_an_error() {
        let dir = crate::testsupport::unique_temp_dir("mev-lane-discover-slug-collision");
        write(
            &dir,
            "planning/roadmaps/alpha/lane-substrate.txt",
            "MV.ticket.a\n",
        );
        write(&dir, "planning/alpha/lane-gtm.txt", "OK.ticket.b\n");

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
        let dir = crate::testsupport::unique_temp_dir("mev-lane-discover-non-roadmap-legacy");
        write(
            &dir,
            "planning/roadmaps/alpha/lane-substrate.txt",
            "MV.ticket.a\n",
        );
        write(&dir, "planning/alpha/tasks.md", "not a lane file\n");

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
    fn discover_lane_files_empty_root_returns_nothing() {
        let dir = crate::testsupport::unique_temp_dir("mev-lane-discover-empty");
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
        let dir = crate::testsupport::unique_temp_dir("mev-lane-discover-symlink-root");
        let vault = dir.join("_vault_roadmaps_alpha");
        std::fs::create_dir_all(&vault).unwrap();
        std::fs::write(vault.join("lane-substrate.txt"), "MV.ticket.a\n").unwrap();

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
    fn parse_lane_blocks_on_real_multi_repo_fixture_matches_hand_checked_ids() {
        // A hand-checked multi-repo fixture, in the spirit of the live
        // close-the-loop/lane-substrate.txt file (base-template, mev, engine-rs, ...).
        let content = "\
# Lane SUBSTRATE
BT.ticket.generate-tasks-json-on-ticket
BT.ticket.compilable-task-boundaries
MV.ticket.learn-link-mapping-masks-dead-links
MV.ticket.close-stale-conformance-branch
OK.ticket.conformance-field-count-floor
CC.ticket.publish-to-crates-io
EN.ticket.stale-generate-timeout-caveat
BA.ticket.spawn-schedule-loop
OR.ticket.publishable-eval-report
";
        let blocks = parse_lane_blocks(content);
        let ids: Vec<&str> = blocks.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "BT.ticket.generate-tasks-json-on-ticket",
                "BT.ticket.compilable-task-boundaries",
                "MV.ticket.learn-link-mapping-masks-dead-links",
                "MV.ticket.close-stale-conformance-branch",
                "OK.ticket.conformance-field-count-floor",
                "CC.ticket.publish-to-crates-io",
                "EN.ticket.stale-generate-timeout-caveat",
                "BA.ticket.spawn-schedule-loop",
                "OR.ticket.publishable-eval-report",
            ]
        );
    }

    // -- Task 2: ownership-based segmentation --------------------------------------

    fn refs(ids: &[&str]) -> Vec<LaneBlockRef> {
        ids.iter()
            .enumerate()
            .map(|(i, id)| LaneBlockRef {
                id: id.to_string(),
                line: i + 1,
                origin: None,
            })
            .collect()
    }

    /// A resolver closure over a fixed `id -> repo` table, for tests that don't need
    /// the full corpus-wide [`OwnerIndex`] machinery.
    fn table_resolver(
        table: &'static [(&'static str, &'static str)],
    ) -> impl FnMut(&str) -> Option<String> {
        move |id: &str| {
            table
                .iter()
                .find(|(k, _)| *k == id)
                .map(|(_, repo)| repo.to_string())
        }
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
        // Unknown to the corpus entirely — also None (Task 3's diagnostic case).
        assert_eq!(resolve_owner(&index, "NOPE.id"), None);
    }

    #[test]
    fn segment_lane_blocks_cuts_at_every_ownership_change() {
        let blocks = refs(&["A.1", "A.2", "B.1", "C.1", "C.2", "C.3"]);
        let segments = segment_lane_blocks(
            &blocks,
            table_resolver(&[
                ("A.1", "repoA"),
                ("A.2", "repoA"),
                ("B.1", "repoB"),
                ("C.1", "repoC"),
                ("C.2", "repoC"),
                ("C.3", "repoC"),
            ]),
        );

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
        let blocks = refs(&["A.1", "B.1", "A.2"]);
        let segments = segment_lane_blocks(
            &blocks,
            table_resolver(&[("A.1", "repoA"), ("B.1", "repoB"), ("A.2", "repoA")]),
        );

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
    fn segment_lane_blocks_omits_unresolvable_ids_without_aborting() {
        let blocks = refs(&["A.1", "GHOST.id", "A.2"]);
        let segments = segment_lane_blocks(
            &blocks,
            table_resolver(&[("A.1", "repoA"), ("A.2", "repoA")]),
        );

        // The unresolvable block contributes no cut and appears in no segment; the
        // two resolvable blocks still land in one contiguous repoA segment.
        assert_eq!(segments.len(), 1, "expected 1 segment, got {segments:?}");
        assert_eq!(segments[0].blocks.len(), 2);
        let ids: Vec<&str> = segments[0].blocks.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(ids, vec!["A.1", "A.2"]);
    }

    #[test]
    fn segment_lane_file_attaches_roadmap_and_lane_to_every_position() {
        let lane_file = LaneFile {
            roadmap: "close-the-loop".to_string(),
            lane: "substrate".to_string(),
            path: PathBuf::from("planning/close-the-loop/lane-substrate.txt"),
            blocks: refs(&["A.1", "B.1", "B.2"]),
        };
        let positions = segment_lane_file(
            &lane_file,
            table_resolver(&[("A.1", "repoA"), ("B.1", "repoB"), ("B.2", "repoB")]),
        );

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

    // -- Task 3: unresolvable IDs are a diagnostic, never a guess --------------------

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
    fn unresolved_owner_diagnostics_fires_on_unknown_id_and_does_not_abort() {
        let lane_file = LaneFile {
            roadmap: "close-the-loop".to_string(),
            lane: "substrate".to_string(),
            path: PathBuf::from("planning/close-the-loop/lane-substrate.txt"),
            blocks: refs(&["A.1", "GHOST.id", "A.2"]),
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
        assert_eq!(diags[0].locator, "2"); // GHOST.id is the second block, line 2
        assert!(diags[0].message.contains("GHOST.id"));
        assert!(diags[0].message.contains("unknown to the corpus"));

        // Segmentation over the same data still proceeds, omitting the unresolvable
        // block rather than aborting — the diagnostic and the derivation are separate
        // concerns.
        let positions = segment_lane_file(&lane_file, |id| {
            index.get(id).filter(|r| r.len() == 1).map(|r| r[0].clone())
        });
        let ids: Vec<&str> = positions.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, vec!["A.1", "A.2"]);
    }

    #[test]
    fn unresolved_owner_diagnostics_fires_on_ambiguous_multi_repo_id() {
        let lane_file = LaneFile {
            roadmap: "close-the-loop".to_string(),
            lane: "substrate".to_string(),
            path: PathBuf::from("planning/close-the-loop/lane-substrate.txt"),
            blocks: refs(&["SHARED.id"]),
        };
        let index = owner_index_from(&[("SHARED.id", &["alpha", "beta"])]);

        let diags = unresolved_owner_diagnostics(&lane_file, &index);
        assert_eq!(
            diags.len(),
            1,
            "expected exactly one diagnostic, got {diags:?}"
        );
        assert_eq!(diags[0].severity, crate::Severity::Warning);
        assert!(diags[0].message.contains("SHARED.id"));
        assert!(diags[0].message.contains("ambiguous"));
        assert!(diags[0].message.contains("alpha"));
        assert!(diags[0].message.contains("beta"));
    }

    #[test]
    fn unresolved_owner_diagnostics_empty_when_everything_resolves() {
        let lane_file = LaneFile {
            roadmap: "close-the-loop".to_string(),
            lane: "substrate".to_string(),
            path: PathBuf::from("planning/close-the-loop/lane-substrate.txt"),
            blocks: refs(&["A.1", "B.1"]),
        };
        let index = owner_index_from(&[("A.1", &["repoA"]), ("B.1", &["repoB"])]);

        let diags = unresolved_owner_diagnostics(&lane_file, &index);
        assert!(diags.is_empty(), "expected no diagnostics, got {diags:?}");
    }

    // -- Task 4: double-claim validation error and `# ORIGIN:` -----------------------

    #[test]
    fn parse_lane_blocks_origin_none_attaches_to_next_block_only() {
        let content = "\
HQ.ticket.before

# ORIGIN: none -- standalone chore, track \"Chores\" (wave 462), not a roadmap block.
#   Adopted into this lane by the operator, sequenced here for scheduling reasons only.
HQ.chore.adopted

HQ.ticket.after
";
        let blocks = parse_lane_blocks(content);
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0].id, "HQ.ticket.before");
        assert_eq!(blocks[0].origin, None);
        assert_eq!(blocks[1].id, "HQ.chore.adopted");
        assert_eq!(blocks[1].origin, Some(Origin::None));
        // The annotation must not leak onto the block after the one it targets.
        assert_eq!(blocks[2].id, "HQ.ticket.after");
        assert_eq!(blocks[2].origin, None);
    }

    #[test]
    fn parse_lane_blocks_origin_roadmap_path_extracts_slug() {
        let content = "\
# ORIGIN: /Users/brandon/Dev/agentic-portfolio/planning/operator-in-the-loop/roadmap.md
#         (the adopted block below -- its run-record ledger row carries
#          origin_roadmap: operator-in-the-loop per D57 section 3)
OK.ticket.operator-edge-types

MV.ticket.operator-edge-graph
";
        let blocks = parse_lane_blocks(content);
        assert_eq!(blocks.len(), 2);
        assert_eq!(
            blocks[0].origin,
            Some(Origin::Roadmap("operator-in-the-loop".to_string()))
        );
        // Only the single next block gets it, even though the human-facing prose talks
        // about it as if it could cover more.
        assert_eq!(blocks[1].origin, None);
    }

    #[test]
    fn parse_lane_blocks_repo_boundary_prose_in_comment_is_inert() {
        // AC 4: no comment other than `# ORIGIN:` is parsed for structure. Prose that
        // reads exactly like a directive must change nothing.
        let content = "\
# repo: fake-owner claims everything below this line
# ROADMAP-BOUNDARY: treat the rest of this file as belonging to repoX
MV.ticket.one
BT.ticket.two
";
        let blocks = parse_lane_blocks(content);
        let ids: Vec<&str> = blocks.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(ids, vec!["MV.ticket.one", "BT.ticket.two"]);
        assert!(blocks.iter().all(|b| b.origin.is_none()));
    }

    fn lane_file(roadmap: &str, lane: &str, path: &str, blocks: Vec<LaneBlockRef>) -> LaneFile {
        LaneFile {
            roadmap: roadmap.to_string(),
            lane: lane.to_string(),
            path: PathBuf::from(path),
            blocks,
        }
    }

    #[test]
    fn resolve_double_claims_single_roadmap_reuse_is_not_a_double_claim() {
        // The same ID repeated across two lane files that both belong to ONE roadmap is
        // ordinary reuse, not a double-claim — resolve_double_claims must ignore it.
        let files = vec![
            lane_file(
                "close-the-loop",
                "substrate",
                "lane-substrate.txt",
                refs(&["MV.ticket.shared"]),
            ),
            lane_file(
                "close-the-loop",
                "cockpit",
                "lane-cockpit.txt",
                refs(&["MV.ticket.shared"]),
            ),
        ];
        let (resolutions, diags) = resolve_double_claims(&files);
        assert!(
            resolutions.is_empty(),
            "expected no resolutions, got {resolutions:?}"
        );
        assert!(diags.is_empty(), "expected no diagnostics, got {diags:?}");
    }

    #[test]
    fn resolve_double_claims_unannotated_cross_roadmap_claim_is_an_error() {
        let files = vec![
            lane_file(
                "operator-surface",
                "substrate",
                "planning/operator-surface/lane-substrate.txt",
                refs(&["OK.ticket.operator-edge-types"]),
            ),
            lane_file(
                "operator-in-the-loop",
                "brain",
                "planning/operator-in-the-loop/lane-brain.txt",
                refs(&["OK.ticket.operator-edge-types"]),
            ),
        ];
        let (resolutions, diags) = resolve_double_claims(&files);
        assert!(
            resolutions.is_empty(),
            "an unannotated double-claim must never resolve, got {resolutions:?}"
        );
        assert_eq!(
            diags.len(),
            1,
            "expected exactly one diagnostic, got {diags:?}"
        );
        assert_eq!(diags[0].severity, crate::Severity::Error);
        assert_eq!(diags[0].locator, "E_LANE_DOUBLE_CLAIM");
        assert!(diags[0].message.contains("OK.ticket.operator-edge-types"));
        assert!(diags[0].message.contains("operator-surface"));
        assert!(diags[0].message.contains("operator-in-the-loop"));
    }

    #[test]
    fn resolve_double_claims_origin_resolves_to_the_annotated_claim() {
        let mut annotated = refs(&["OK.ticket.operator-edge-types"]);
        annotated[0].origin = Some(Origin::Roadmap("operator-in-the-loop".to_string()));

        let files = vec![
            lane_file(
                "operator-surface",
                "substrate",
                "planning/operator-surface/lane-substrate.txt",
                annotated,
            ),
            lane_file(
                "operator-in-the-loop",
                "brain",
                "planning/operator-in-the-loop/lane-brain.txt",
                refs(&["OK.ticket.operator-edge-types"]),
            ),
        ];
        let (resolutions, diags) = resolve_double_claims(&files);
        assert!(diags.is_empty(), "expected no diagnostics, got {diags:?}");
        assert_eq!(
            resolutions.len(),
            1,
            "expected one resolution, got {resolutions:?}"
        );
        assert_eq!(resolutions[0].id, "OK.ticket.operator-edge-types");
        // Executes under operator-surface (the annotated claim), not operator-in-the-loop.
        assert_eq!(resolutions[0].roadmap, "operator-surface");
        assert_eq!(resolutions[0].lane, "substrate");
        assert_eq!(
            resolutions[0].origin_roadmap,
            Some("operator-in-the-loop".to_string())
        );
    }

    #[test]
    fn resolve_double_claims_both_annotated_is_ambiguous_error() {
        let mut a = refs(&["MV.ticket.shared"]);
        a[0].origin = Some(Origin::Roadmap("roadmap-b".to_string()));
        let mut b = refs(&["MV.ticket.shared"]);
        b[0].origin = Some(Origin::Roadmap("roadmap-a".to_string()));

        let files = vec![
            lane_file("roadmap-a", "lane-a", "lane-a.txt", a),
            lane_file("roadmap-b", "lane-b", "lane-b.txt", b),
        ];
        let (resolutions, diags) = resolve_double_claims(&files);
        assert!(
            resolutions.is_empty(),
            "two competing ORIGIN annotations must never resolve, got {resolutions:?}"
        );
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].severity, crate::Severity::Error);
        assert_eq!(diags[0].locator, "E_LANE_DOUBLE_CLAIM");
    }

    #[test]
    fn derive_lane_positions_resolved_double_claim_renders_once_with_origin_roadmap() {
        let mut annotated = refs(&["OK.ticket.operator-edge-types"]);
        annotated[0].origin = Some(Origin::Roadmap("operator-in-the-loop".to_string()));

        let files = vec![
            lane_file(
                "operator-surface",
                "substrate",
                "planning/operator-surface/lane-substrate.txt",
                annotated,
            ),
            lane_file(
                "operator-in-the-loop",
                "brain",
                "planning/operator-in-the-loop/lane-brain.txt",
                refs(&["OK.ticket.operator-edge-types"]),
            ),
        ];
        let (positions, diags) = derive_lane_positions(&files, |_| Some("okf-core".to_string()));
        assert!(diags.is_empty(), "expected no diagnostics, got {diags:?}");
        assert_eq!(
            positions.len(),
            1,
            "the block must render exactly once, got {positions:?}"
        );
        assert_eq!(positions[0].roadmap, "operator-surface");
        assert_eq!(positions[0].lane, "substrate");
        assert_eq!(
            positions[0].origin_roadmap,
            Some("operator-in-the-loop".to_string())
        );
    }

    #[test]
    fn derive_lane_positions_unannotated_double_claim_renders_nowhere() {
        let files = vec![
            lane_file(
                "operator-surface",
                "substrate",
                "planning/operator-surface/lane-substrate.txt",
                refs(&["OK.ticket.operator-edge-types"]),
            ),
            lane_file(
                "operator-in-the-loop",
                "brain",
                "planning/operator-in-the-loop/lane-brain.txt",
                refs(&["OK.ticket.operator-edge-types"]),
            ),
        ];
        let (positions, diags) = derive_lane_positions(&files, |_| Some("okf-core".to_string()));
        assert_eq!(
            diags.len(),
            1,
            "expected the double-claim error, got {diags:?}"
        );
        assert!(
            positions.is_empty(),
            "an unresolved double-claim must render in neither lane, got {positions:?}"
        );
    }

    #[test]
    fn derive_lane_positions_ordinary_blocks_unaffected_by_double_claim_machinery() {
        let files = vec![lane_file(
            "close-the-loop",
            "substrate",
            "lane-substrate.txt",
            refs(&["MV.ticket.a", "BT.ticket.b"]),
        )];
        let (positions, diags) = derive_lane_positions(&files, |id| {
            if id.starts_with("MV") {
                Some("mev".to_string())
            } else {
                Some("base-template".to_string())
            }
        });
        assert!(diags.is_empty());
        assert_eq!(positions.len(), 2);
        assert!(positions.iter().all(|p| p.origin_roadmap.is_none()));
    }

    // -----------------------------------------------------------------------
    // plan_lane_segments — MV.13.A Task 5
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
        let dir = crate::testsupport::unique_temp_dir("mev-plan-lane-segments-basic");
        write(
            &dir,
            "planning/roadmaps/alpha/lane-substrate.txt",
            "MV.ticket.a\nBT.ticket.b\n",
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
        assert_eq!(blocks[1]["repo"], "base-template");
        assert_eq!(blocks[1]["segment"], 1);
        assert_eq!(blocks[1]["position"], 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn plan_lane_segments_no_lane_files_plans_nothing() {
        let dir = crate::testsupport::unique_temp_dir("mev-plan-lane-segments-empty");
        std::fs::create_dir_all(dir.join("planning")).unwrap();

        let plan = plan_lane_segments(&dir, &[]);
        assert!(plan.actions.is_empty());
        assert!(plan.diagnostics.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn plan_lane_segments_unresolved_id_surfaces_a_diagnostic() {
        let dir = crate::testsupport::unique_temp_dir("mev-plan-lane-segments-unresolved");
        write(
            &dir,
            "planning/roadmaps/alpha/lane-substrate.txt",
            "MV.ticket.unknown\n",
        );

        let plan = plan_lane_segments(&dir, &[]);
        assert!(
            plan.diagnostics
                .iter()
                .any(|d| d.message.contains("could not resolve an owner")),
            "expected an unresolved-owner diagnostic, got {:?}",
            plan.diagnostics
        );
        // Still writes the artifact — a diagnostic is not an abort.
        assert_eq!(plan.actions.len(), 1);
        let artifact: serde_json::Value =
            serde_json::from_str(&plan.actions[0].new_content).unwrap();
        assert!(artifact["blocks"].as_array().unwrap().is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------------
    // roadmap_slug_from_plan_path / derive_program_membership — MV.13.D Task 3
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
        let dir = crate::testsupport::unique_temp_dir("mev-derive-program-membership-basic");
        write(
            &dir,
            "planning/roadmaps/alpha/lane-substrate.txt",
            "MV.ticket.a\nBT.ticket.b\n",
        );
        let loaded = vec![
            state_file_fixture("mev", "MV.ticket.a"),
            state_file_fixture("base-template", "BT.ticket.b"),
        ];

        let by_roadmap = derive_program_membership(&dir, &loaded);
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
        let dir = crate::testsupport::unique_temp_dir("mev-derive-program-membership-empty");
        std::fs::create_dir_all(dir.join("planning")).unwrap();

        let by_roadmap = derive_program_membership(&dir, &[]);
        assert!(by_roadmap.is_empty(), "got {by_roadmap:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
