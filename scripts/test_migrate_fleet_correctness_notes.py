#!/usr/bin/env python3
"""
Tests for scripts/migrate_fleet_correctness_notes.py.

Run directly: `python3 scripts/test_migrate_fleet_correctness_notes.py`.

Covers, per the block's testing_strategy and this task's acceptance
criteria:
  (a) a `note` with a recognized F0-F3 prefix is lifted into
      `fleet_correctness` and the prefix is stripped from `note`;
  (b) a `note` with no F prefix is left ungraded AND reported (never
      guessed at);
  (c) an already-graded record (typed field already set) is not
      double-migrated — its `note` is left untouched even if it still
      carries an F-prefix;
  (d) an unrecognized prefix (`F9:`) is reported, not coerced into any
      known grade;
  (e) THE POSITIVE CONTROL — a byte-for-byte round-trip of a fixture
      containing an em dash, proving `ensure_ascii=False` is actually in
      effect (the stdlib default, `ensure_ascii=True`, would escape it);
  (f) --write is required to persist anything; the default (dry-run)
      changes nothing on disk;
  (g) an unchanged file is never reopened for writing at all (so its
      exact bytes, whitespace included, survive a --write run untouched).
"""

from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import migrate_fleet_correctness_notes as mig  # noqa: E402


def make_state(blocks: list[dict], carryover: list[dict] | None = None) -> dict:
    return {
        "repo": "fix",
        "kind": "project",
        "updated": "2026-09-06",
        "focus": {"now": [], "next": [], "blocked": []},
        "tracks": [{"title": "T", "blocks": blocks}],
        "carryover": carryover or [],
    }


class ExtractGradeTests(unittest.TestCase):
    def test_recognized_prefix_is_extracted_and_stripped(self):
        self.assertEqual(
            mig.extract_grade("F1: some caveat text"),
            ("F1", "some caveat text"),
        )

    def test_unrecognized_digit_prefix_still_matches_as_a_grade_token(self):
        # F9 is not a KNOWN grade, but it must still be recognized as a
        # grade-shaped prefix (so it lands in "unrecognized", not silently
        # in the no-prefix-at-all ungraded bucket).
        self.assertEqual(mig.extract_grade("F9: odd one"), ("F9", "odd one"))

    def test_no_prefix_returns_none(self):
        self.assertIsNone(mig.extract_grade("just a plain note"))

    def test_none_note_returns_none(self):
        self.assertIsNone(mig.extract_grade(None))

    def test_prefix_not_at_start_is_not_matched(self):
        self.assertIsNone(mig.extract_grade("see F1: elsewhere"))


class MigrateStateDataTests(unittest.TestCase):
    def test_recognized_prefix_is_lifted_and_note_prefix_stripped(self):
        data = make_state(
            [{"id": "A", "title": "A", "note": "F0: money on fire"}]
        )
        result = mig.migrate_state_data("fix", Path("state.json"), data)

        self.assertEqual(len(result.lifted), 1)
        self.assertEqual(result.lifted[0].prefix, "F0")
        self.assertTrue(result.changed)
        block = data["tracks"][0]["blocks"][0]
        self.assertEqual(block["fleet_correctness"], "F0")
        self.assertEqual(block["note"], "money on fire")

    def test_prefix_with_nothing_left_over_clears_the_note_key_entirely(self):
        data = make_state([{"id": "A", "title": "A", "note": "F2:"}])
        mig.migrate_state_data("fix", Path("state.json"), data)
        block = data["tracks"][0]["blocks"][0]
        self.assertEqual(block["fleet_correctness"], "F2")
        self.assertNotIn("note", block)

    def test_note_with_no_f_prefix_is_left_ungraded_and_reported(self):
        data = make_state(
            [{"id": "A", "title": "A", "note": "just some context, no grade"}]
        )
        result = mig.migrate_state_data("fix", Path("state.json"), data)

        self.assertEqual(result.lifted, [])
        self.assertEqual(result.ungraded_block_count, 1)
        self.assertFalse(result.changed)
        block = data["tracks"][0]["blocks"][0]
        self.assertNotIn("fleet_correctness", block)
        self.assertEqual(block["note"], "just some context, no grade")

    def test_absent_note_produces_no_finding_at_all(self):
        data = make_state([{"id": "A", "title": "A"}])
        result = mig.migrate_state_data("fix", Path("state.json"), data)

        self.assertEqual(result.lifted, [])
        self.assertEqual(result.unrecognized, [])
        self.assertEqual(result.ungraded_block_count, 0)
        self.assertFalse(result.changed)

    def test_already_graded_record_is_not_double_migrated(self):
        data = make_state(
            [
                {
                    "id": "A",
                    "title": "A",
                    "note": "F1: stale prefix left behind",
                    "fleet_correctness": "F3",
                }
            ]
        )
        result = mig.migrate_state_data("fix", Path("state.json"), data)

        self.assertEqual(result.lifted, [])
        self.assertEqual(len(result.already_graded), 1)
        self.assertFalse(result.changed)
        block = data["tracks"][0]["blocks"][0]
        # Untouched: the typed field wins, and the note is left exactly as
        # authored rather than second-guessed.
        self.assertEqual(block["fleet_correctness"], "F3")
        self.assertEqual(block["note"], "F1: stale prefix left behind")

    def test_unrecognized_prefix_is_reported_not_coerced(self):
        data = make_state([{"id": "A", "title": "A", "note": "F9: mystery grade"}])
        result = mig.migrate_state_data("fix", Path("state.json"), data)

        self.assertEqual(result.lifted, [])
        self.assertEqual(len(result.unrecognized), 1)
        self.assertEqual(result.unrecognized[0].prefix, "F9")
        self.assertFalse(result.unrecognized[0].known)
        self.assertFalse(result.changed)
        block = data["tracks"][0]["blocks"][0]
        self.assertNotIn("fleet_correctness", block)
        self.assertEqual(block["note"], "F9: mystery grade")

    def test_carryover_text_is_migrated_by_the_same_rules_as_note(self):
        data = make_state(
            [],
            carryover=[
                {
                    "slug": "c1",
                    "scope": {"repo": "fix", "tier": None, "cross_repo": None},
                    "kind": "deferred",
                    "text": "F0: expensive to leave open",
                }
            ],
        )
        result = mig.migrate_state_data("fix", Path("state.json"), data)

        self.assertEqual(len(result.lifted), 1)
        self.assertEqual(result.lifted[0].container, "carryover")
        item = data["carryover"][0]
        self.assertEqual(item["fleet_correctness"], "F0")
        self.assertEqual(item["text"], "expensive to leave open")

    def test_carryover_with_no_prefix_is_reported_ungraded_not_touched(self):
        data = make_state(
            [],
            carryover=[
                {
                    "slug": "c1",
                    "scope": {"repo": "fix", "tier": None, "cross_repo": None},
                    "kind": "deferred",
                    "text": "no grade here",
                }
            ],
        )
        result = mig.migrate_state_data("fix", Path("state.json"), data)
        self.assertEqual(result.lifted, [])
        self.assertEqual(result.ungraded_carryover_count, 1)
        self.assertFalse(result.changed)

    def test_candidates_counts_lifted_plus_unrecognized_plus_already_graded(self):
        data = make_state(
            [
                {"id": "A", "title": "A", "note": "F0: lift me"},
                {"id": "B", "title": "B", "note": "F9: unknown"},
                {
                    "id": "C",
                    "title": "C",
                    "note": "F1: stale",
                    "fleet_correctness": "F1",
                },
                {"id": "D", "title": "D", "note": "no grade at all"},
            ]
        )
        result = mig.migrate_state_data("fix", Path("state.json"), data)
        self.assertEqual(len(result.candidates("block")), 3)
        self.assertEqual(result.ungraded_block_count, 1)


class RoundTripTests(unittest.TestCase):
    """The positive control: proves `ensure_ascii=False` is actually in
    effect, and that an unchanged file is never touched on disk."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        (self.root / "planning").mkdir(parents=True)
        (self.root / "brain.toml").write_text(
            "[[repos]]\n"
            'slug = "fix"\n'
            'prefix = "FX"\n'
            'tier = "_root"\n'
            'repo_path = "."\n',
            encoding="utf-8",
        )

    def _write_state(self, data: dict) -> Path:
        path = self.root / "planning" / "state.json"
        path.write_text(
            json.dumps(data, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
        )
        return path

    def test_em_dash_survives_a_real_write_byte_for_byte_apart_from_the_change(self):
        data = make_state(
            [
                {
                    "id": "A",
                    "title": "A",
                    "note": "F1: risk — the blast radius is fleet-wide",
                }
            ]
        )
        path = self._write_state(data)
        before = path.read_text(encoding="utf-8")
        self.assertIn("—", before)  # em dash present pre-migration

        results = mig.run(self.root, None, write=True)
        self.assertEqual(len(results), 1)
        self.assertTrue(results[0].changed)

        after = path.read_text(encoding="utf-8")
        # The em dash must survive verbatim, not become `—` or `?`.
        self.assertIn("—", after)
        self.assertNotIn("\\u2014", after)
        reloaded = json.loads(after)
        block = reloaded["tracks"][0]["blocks"][0]
        self.assertEqual(block["fleet_correctness"], "F1")
        self.assertEqual(block["note"], "risk — the blast radius is fleet-wide")
        # Byte-for-byte apart from the intended change: re-dumping the
        # migrated in-memory structure the same way reproduces the file
        # exactly, proving no incidental reformatting crept in.
        expected = json.dumps(reloaded, indent=2, ensure_ascii=False) + "\n"
        self.assertEqual(after, expected)

    def test_dry_run_default_writes_nothing_to_disk(self):
        data = make_state([{"id": "A", "title": "A", "note": "F0: lift me"}])
        path = self._write_state(data)
        before_bytes = path.read_bytes()

        results = mig.run(self.root, None, write=False)
        self.assertTrue(results[0].changed)  # would change, but didn't

        after_bytes = path.read_bytes()
        self.assertEqual(before_bytes, after_bytes)

    def test_unchanged_file_is_never_reopened_for_writing(self):
        data = make_state([{"id": "A", "title": "A", "note": "no grade here"}])
        path = self._write_state(data)
        before_mtime_ns = path.stat().st_mtime_ns
        before_bytes = path.read_bytes()

        results = mig.run(self.root, None, write=True)
        self.assertFalse(results[0].changed)

        self.assertEqual(path.read_bytes(), before_bytes)
        self.assertEqual(path.stat().st_mtime_ns, before_mtime_ns)


class FindHqRootTests(unittest.TestCase):
    def test_finds_brain_toml_in_a_parent_directory(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "brain.toml").write_text("", encoding="utf-8")
            nested = root / "core" / "mev" / "scripts"
            nested.mkdir(parents=True)
            self.assertEqual(mig.find_hq_root(nested), root)

    def test_raises_when_no_brain_toml_is_found(self):
        with tempfile.TemporaryDirectory() as tmp:
            # A bare temp dir's parents (system tmp root, "/") will not
            # carry a brain.toml either, so this reliably raises.
            with self.assertRaises(mig.MigrationError):
                mig.find_hq_root(Path(tmp) / "deep" / "nowhere")


if __name__ == "__main__":
    unittest.main()
