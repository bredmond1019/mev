#!/usr/bin/env python3
"""Report whether the last consumer-compile run actually verified every consumer.

Reads the result file `scripts/check_consumers.sh` writes on every run
(`target/consumer-coverage.json`) and reports coverage as a distinct verdict,
rather than re-running the compile — that gate is `perTask: false` precisely
because it runs two real cold cargo builds over other repos, and answering a
question it already answered should not cost that again.

Exit codes, matching the three-verdict design of
`MV.ticket.consumer-gate-must-report-its-real-coverage`:
  0 — every discovered consumer was verified
  4 — coverage is UNKNOWN: incomplete, or the result file is absent or stale

Exit 4 is deliberately not 1: nothing is broken, something was not checked, and
the two must stay distinguishable. This check is registered `gates: false`, so a
4 reports without red-gating a lane whose only sin is a dirty sibling tree.

Lives in a file rather than inline in `planning/harness.json` because an
836-character embedded program with 18 newlines in a `command` field made that
file unreadable to the SDLC engine's config stage (HARNESS_CONFIG_UNPARSEABLE,
2026-09-07) even though it was valid JSON to both python and node. Every other
check in this repo is a short invocation; this one now is too.
"""
import json
import subprocess
import sys
from pathlib import Path

RESULT = Path("target/consumer-coverage.json")


def main() -> int:
    try:
        data = json.loads(RESULT.read_text())
    except Exception as exc:  # absent, unreadable, or malformed — all "unknown"
        print(
            f"consumer-coverage: result file absent or unreadable ({exc}) — "
            "coverage unknown, run scripts/check_consumers.sh"
        )
        return 4

    head = subprocess.run(
        ["git", "rev-parse", "HEAD"], capture_output=True, text=True
    ).stdout.strip()
    sha = data.get("git_sha")
    if sha != head:
        print(
            f"consumer-coverage: result file is stale (recorded {sha}, HEAD is {head}) "
            "— coverage unknown"
        )
        return 4

    verified = data.get("verified_count", 0)
    total = data.get("total_count", 0)
    if verified < total:
        unverified = [
            f"{c.get('slug')}: {c.get('outcome')}"
            for c in data.get("consumers", [])
            if c.get("outcome") != "pass"
        ]
        detail = f" ({', '.join(unverified)})" if unverified else ""
        print(f"consumer-coverage: incomplete ({verified} of {total} verified){detail}")
        return 4

    print(f"consumer-coverage: complete ({verified} of {total} verified, sha {head})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
