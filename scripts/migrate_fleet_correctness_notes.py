#!/usr/bin/env python3
"""
migrate_fleet_correctness_notes.py — one-time backfill lifting D80's
`F0:`-`F3:` fleet-correctness prefixes out of block `note` (and, for
symmetry, carryover `text`) into the typed `fleet_correctness` field
`OK.ticket.track-block-gains-a-fleet-correctness-field` landed in okf-core.

D80 specified the `note`/`text` prefix as the interim home for this grade
while no typed field existed, so the grades already written there are the
AUTHORITATIVE source — a human spent a review pass assigning them. This
script is therefore a read-then-verify migration, never a regex-and-hope:
every record it cannot confidently classify is REPORTED, not guessed at,
and an already-graded record is never touched a second time.

Measured 2026-09-06 across all fleet `planning/state.json` files: block
`note` carries 53 F0:-F3: prefixed entries; carryover `text` carries ZERO
(positive control: the identical scan over block `note` in the same pass
returns 53, so the zero is a true negative, not a blind scan). This script
still scans BOTH containers and reports "0 candidates" for carryover
rather than pretending the container does not exist — the filter this
migration feeds (`mev carryover --fleet-correctness`) is real even though
it is empty today.

--dry-run is the DEFAULT and prints a per-repo report without touching
any file on disk; pass --write to actually persist the migration. This is
not optional caution: `state.json` files are the single most contended
artifact in this fleet, concurrent lanes are live, and re-serialising one
rewrites the whole file. A file this migration does not change is never
reopened for writing at all, so it is trivially byte-identical; a file
that IS changed is written with `json.dump(..., indent=2,
ensure_ascii=False)` plus a trailing newline — the fleet's standing
round-trip recipe (CLAUDE.md trap 3). `ensure_ascii=True` (the json
module's default) escapes every em dash and turns a small edit into ~130
lines of churn and a conflict for every sibling lane.

Discovery follows brain.toml's `[[repos]]` table (same source `mev
carryover`/`bastion validate-brain` read) rather than globbing the tree —
CLAUDE.md standing rule 9: `rg`/`find` are symlink-blind, and every
`planning/` in this fleet is a symlink.

Usage:
    python3 scripts/migrate_fleet_correctness_notes.py [--write] [--repo SLUG ...] [--root PATH] [--json]
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path

try:
    import tomllib
except ImportError:  # pragma: no cover - py<3.11 fallback, not expected here
    import tomli as tomllib  # type: ignore

SCRIPT_DIR = Path(__file__).resolve().parent

# The on-disk grade prefix: an uppercase "F" followed by one or more digits
# and a colon, optionally followed by whitespace, at the very START of the
# text. Matching `F(\d+)` rather than a fixed `F[0-3]` is deliberate: an
# out-of-vocabulary grade like `F9:` must still be RECOGNIZED as a grade
# prefix (so it is reported as unrecognized, not silently left in the
# "no prefix at all" ungraded bucket) even though it is never lifted.
GRADE_PREFIX_RE = re.compile(r"^\s*(F(\d+)):\s*")

# The known, in-vocabulary grades (okf_core::KnownFleetCorrectness) — the
# only tokens this script ever lifts into `fleet_correctness`. Anything
# else that still matches GRADE_PREFIX_RE is reported as unrecognized.
KNOWN_GRADES = {"F0", "F1", "F2", "F3"}


class MigrationError(RuntimeError):
    pass


def find_hq_root(start: Path) -> Path:
    """Walk upward from `start` until a directory carrying `brain.toml` is
    found — the fleet HQ root `mev carryover`/`bastion validate-brain` also
    resolve from. Raises if none is found (never guesses a root)."""
    for parent in [start, *start.parents]:
        if (parent / "brain.toml").is_file():
            return parent
    raise MigrationError(
        f"could not locate brain.toml (HQ root) from any parent of {start}"
    )


def load_repos(root: Path) -> dict[str, Path]:
    """slug -> state.json path, exactly as `brain.toml`'s `[[repos]]` table
    declares it (symlink or not) — never a filesystem glob (CLAUDE.md
    standing rule 9)."""
    brain_toml = root / "brain.toml"
    with brain_toml.open("rb") as f:
        data = tomllib.load(f)
    repos: dict[str, Path] = {}
    for entry in data.get("repos", []):
        slug = entry["slug"]
        repo_path = entry["repo_path"]
        state_path = (root / repo_path / "planning" / "state.json").resolve(
            strict=False
        )
        repos[slug] = state_path
    return repos


@dataclass
class Candidate:
    """One record whose `note`/`text` carries a recognized `F<digits>:`
    prefix — regardless of whether it ends up lifted, already-graded, or
    unrecognized."""

    repo: str
    container: str  # "block" or "carryover"
    key: str  # block id, or carryover slug
    prefix: str  # e.g. "F0", "F9"

    @property
    def known(self) -> bool:
        return self.prefix in KNOWN_GRADES


@dataclass
class MigrationResult:
    repo: str
    path: Path
    exists: bool = True
    lifted: list[Candidate] = field(default_factory=list)
    unrecognized: list[Candidate] = field(default_factory=list)
    already_graded: list[Candidate] = field(default_factory=list)
    ungraded_block_count: int = 0
    ungraded_carryover_count: int = 0
    changed: bool = False
    data: dict | None = None

    def candidates(self, container: str) -> list[Candidate]:
        """Every record recognized as carrying an F-prefix in `container`
        ("block" or "carryover"), whether lifted, already-graded, or
        unrecognized — the "candidate" count the AC asks for."""
        return [
            c
            for c in (*self.lifted, *self.unrecognized, *self.already_graded)
            if c.container == container
        ]


def extract_grade(text: object) -> tuple[str, str] | None:
    """`(grade_token, remainder)` if `text` is a string starting with a
    recognized `F<digits>:` prefix, else `None`. `remainder` is the text
    with the prefix (and any immediately-following whitespace) stripped —
    never re-snippeted, never further trimmed beyond that."""
    if not isinstance(text, str):
        return None
    m = GRADE_PREFIX_RE.match(text)
    if not m:
        return None
    return m.group(1), text[m.end() :]


def migrate_state_data(repo: str, path: Path, data: dict) -> MigrationResult:
    """Apply the migration IN MEMORY to `data` (mutated in place) and
    return the result describing what happened. Pure with respect to disk —
    callers decide whether/how to persist `data`."""
    result = MigrationResult(repo=repo, path=path, data=data)

    for track in data.get("tracks", []):
        for block in track.get("blocks", []):
            note = block.get("note")
            already_has_grade = block.get("fleet_correctness") is not None
            extracted = extract_grade(note)
            block_id = block.get("id", "?")

            if extracted is None:
                if isinstance(note, str) and note and not already_has_grade:
                    result.ungraded_block_count += 1
                continue

            grade, remainder = extracted
            cand = Candidate(repo=repo, container="block", key=block_id, prefix=grade)

            if already_has_grade:
                # Never double-migrate: the typed field already carries a
                # verdict, so the note's prefix is left exactly as-is.
                result.already_graded.append(cand)
                continue

            if not cand.known:
                # Reported, never coerced — see module docstring.
                result.unrecognized.append(cand)
                continue

            block["fleet_correctness"] = grade
            remainder = remainder.strip()
            if remainder:
                block["note"] = remainder
            else:
                block.pop("note", None)
            result.lifted.append(cand)
            result.changed = True

    for item in data.get("carryover", []):
        text = item.get("text")
        already_has_grade = item.get("fleet_correctness") is not None
        extracted = extract_grade(text)
        slug = item.get("slug", "?")

        if extracted is None:
            if isinstance(text, str) and text and not already_has_grade:
                result.ungraded_carryover_count += 1
            continue

        grade, remainder = extracted
        cand = Candidate(repo=repo, container="carryover", key=slug, prefix=grade)

        if already_has_grade:
            result.already_graded.append(cand)
            continue

        if not cand.known:
            result.unrecognized.append(cand)
            continue

        item["fleet_correctness"] = grade
        remainder = remainder.strip()
        item["text"] = remainder
        result.lifted.append(cand)
        result.changed = True

    return result


def migrate_state_file(repo: str, path: Path) -> MigrationResult:
    if not path.is_file():
        return MigrationResult(repo=repo, path=path, exists=False)
    with path.open("r", encoding="utf-8") as f:
        data = json.load(f)
    return migrate_state_data(repo, path, data)


def write_state_file(path: Path, data: dict) -> None:
    """The fleet's standing round-trip recipe (CLAUDE.md trap 3):
    `json.dump(..., indent=2, ensure_ascii=False)` plus a trailing newline
    reproduces the file byte-for-byte apart from the intended change.
    `ensure_ascii=True` (the stdlib default) would escape every em dash
    and turn a small edit into ~130 lines of churn."""
    text = json.dumps(data, indent=2, ensure_ascii=False)
    with path.open("w", encoding="utf-8") as f:
        f.write(text)
        f.write("\n")


def _fmt_candidate(c: Candidate) -> str:
    return f"{c.repo}:{c.key} ({c.prefix})"


def render_report(results: list[MigrationResult]) -> str:
    lines: list[str] = []
    total_block_candidates = 0
    total_carryover_candidates = 0
    total_lifted = 0
    total_unrecognized = 0
    total_already_graded = 0
    total_ungraded_block = 0
    total_ungraded_carryover = 0

    for r in results:
        if not r.exists:
            continue
        block_cands = r.candidates("block")
        carryover_cands = r.candidates("carryover")
        total_block_candidates += len(block_cands)
        total_carryover_candidates += len(carryover_cands)
        total_lifted += len(r.lifted)
        total_unrecognized += len(r.unrecognized)
        total_already_graded += len(r.already_graded)
        total_ungraded_block += r.ungraded_block_count
        total_ungraded_carryover += r.ungraded_carryover_count

        if not (block_cands or carryover_cands or r.ungraded_block_count or r.ungraded_carryover_count):
            continue

        lines.append(f"[{r.repo}] {r.path}")
        lines.append(
            f"  block candidates: {len(block_cands)}"
            f" (lifted {sum(1 for c in r.lifted if c.container == 'block')},"
            f" already-graded {sum(1 for c in r.already_graded if c.container == 'block')},"
            f" unrecognized {sum(1 for c in r.unrecognized if c.container == 'block')})"
        )
        lines.append(
            f"  carryover candidates: {len(carryover_cands)}"
            f" (lifted {sum(1 for c in r.lifted if c.container == 'carryover')},"
            f" already-graded {sum(1 for c in r.already_graded if c.container == 'carryover')},"
            f" unrecognized {sum(1 for c in r.unrecognized if c.container == 'carryover')})"
        )
        if r.ungraded_block_count:
            lines.append(f"  ungraded blocks (no F prefix, reported): {r.ungraded_block_count}")
        if r.ungraded_carryover_count:
            lines.append(f"  ungraded carryover (no F prefix, reported): {r.ungraded_carryover_count}")
        for c in r.unrecognized:
            lines.append(f"  UNRECOGNIZED: {_fmt_candidate(c)}")
        if r.changed:
            lines.append(f"  changed: {len(r.lifted)} record(s) would be written")

    lines.append("")
    lines.append("=== TOTALS ===")
    lines.append(f"block candidates:      {total_block_candidates}")
    lines.append(f"carryover candidates:  {total_carryover_candidates}")
    lines.append(f"  lifted:              {total_lifted}")
    lines.append(f"  already-graded:      {total_already_graded}")
    lines.append(f"  unrecognized:        {total_unrecognized}")
    lines.append(f"ungraded blocks (reported):     {total_ungraded_block}")
    lines.append(f"ungraded carryover (reported):  {total_ungraded_carryover}")
    return "\n".join(lines)


def report_dict(results: list[MigrationResult]) -> dict:
    def cand_dict(c: Candidate) -> dict:
        return {"repo": c.repo, "key": c.key, "prefix": c.prefix, "known": c.known}

    repos = []
    totals = {
        "block_candidates": 0,
        "carryover_candidates": 0,
        "lifted": 0,
        "already_graded": 0,
        "unrecognized": 0,
        "ungraded_block": 0,
        "ungraded_carryover": 0,
    }
    for r in results:
        if not r.exists:
            continue
        block_cands = r.candidates("block")
        carryover_cands = r.candidates("carryover")
        totals["block_candidates"] += len(block_cands)
        totals["carryover_candidates"] += len(carryover_cands)
        totals["lifted"] += len(r.lifted)
        totals["already_graded"] += len(r.already_graded)
        totals["unrecognized"] += len(r.unrecognized)
        totals["ungraded_block"] += r.ungraded_block_count
        totals["ungraded_carryover"] += r.ungraded_carryover_count
        repos.append(
            {
                "repo": r.repo,
                "path": str(r.path),
                "block_candidates": len(block_cands),
                "carryover_candidates": len(carryover_cands),
                "lifted": [cand_dict(c) for c in r.lifted],
                "already_graded": [cand_dict(c) for c in r.already_graded],
                "unrecognized": [cand_dict(c) for c in r.unrecognized],
                "ungraded_block_count": r.ungraded_block_count,
                "ungraded_carryover_count": r.ungraded_carryover_count,
                "changed": r.changed,
            }
        )
    return {"repos": repos, "totals": totals}


def run(
    root: Path,
    repo_filter: list[str] | None,
    write: bool,
) -> list[MigrationResult]:
    repos = load_repos(root)
    if repo_filter:
        repos = {slug: path for slug, path in repos.items() if slug in repo_filter}

    results: list[MigrationResult] = []
    for slug, path in sorted(repos.items()):
        result = migrate_state_file(slug, path)
        results.append(result)
        if write and result.exists and result.changed:
            write_state_file(result.path, result.data)  # type: ignore[arg-type]

    return results


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--write",
        action="store_true",
        help="Actually write the migrated state.json files (default: dry-run, changes nothing on disk)",
    )
    parser.add_argument(
        "--repo",
        action="append",
        default=None,
        help="Limit to this repo slug (repeatable); default is every repo in brain.toml",
    )
    parser.add_argument(
        "--root",
        type=Path,
        default=None,
        help="HQ root containing brain.toml (default: auto-discovered upward from this script)",
    )
    parser.add_argument(
        "--json",
        action="store_true",
        help="Emit a machine-readable JSON report instead of the human-readable one",
    )
    args = parser.parse_args(argv)

    try:
        root = args.root or find_hq_root(SCRIPT_DIR)
        results = run(root, args.repo, args.write)
    except MigrationError as e:
        print(f"error: {e}", file=sys.stderr)
        return 1

    if args.json:
        print(json.dumps(report_dict(results), indent=2))
    else:
        mode = "WRITE" if args.write else "DRY-RUN"
        print(f"mode: {mode}")
        print(render_report(results))

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
