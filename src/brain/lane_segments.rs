//! Lane file discovery and parsing — `MV.13.A` Task 1.
//!
//! A "lane file" (`lane-*.txt`) is the authored, human-readable execution order for one
//! roadmap's slice of work: an ordered list of block IDs, one per line, with `#` comments
//! carrying binding context for the operator running it (`/begin-orchestration` Step 1F).
//! No service reads that structure today — this module is the first thing that turns a
//! lane file into a derived object another consumer (Task 2's segmentation, `emit-state`)
//! can act on.
//!
//! # Comments are opaque
//!
//! `#` comments are stripped when extracting the block-ID list and are never parsed for
//! structure — they are prose for the human, not directives for this walker. The single
//! exception, `# ORIGIN:`, is a declared machine-readable directive and is handled by a
//! later task (MV.13.A Task 4), not here; this module drops it like any other comment.
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
pub fn parse_lane_blocks(content: &str) -> Vec<LaneBlockRef> {
    let mut out = Vec::new();
    for (idx, raw_line) in content.lines().enumerate() {
        let line = idx + 1;
        let before_comment = match raw_line.find('#') {
            Some(pos) => &raw_line[..pos],
            None => raw_line,
        };
        let id = before_comment.trim();
        if id.is_empty() {
            continue;
        }
        out.push(LaneBlockRef {
            id: id.to_string(),
            line,
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
}
