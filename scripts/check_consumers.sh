#!/usr/bin/env bash
# check_consumers.sh — consumer compile gate for mev.
#
# mev sits in the middle of the fleet's Cargo path-dependency graph:
# okf-core -> mev -> bastion / engine-rs. `mev check-consumers` already
# discovers those consumers from brain.toml, compiles their TEST targets
# with `cargo nextest run --no-run --locked` into a fresh
# CARGO_TARGET_DIR, short-circuits a dirty consumer, refuses a verdict if
# Cargo.lock moved, and classifies the result by stderr signature rather
# than exit code (exit 101 has been observed for both a real break and a
# stale lock). This script does NOT re-derive any of that — discovery,
# spawning, lockfile hashing and classification all stay in
# src/consumers/mod.rs and src/lib.rs's check_consumers. This wrapper's
# only job is to invoke the tool, apply the waiver list, and decide the
# exit code (MV.18.A task 2 finishes that part).
#
# Waiver handling (task 1, this file): scripts/consumer-gate-waivers.txt
# names a consumer that is knowingly broken so the gate stays green while
# the fix lives in another lane's repo. Every waiver row must carry a
# slug, an owning block id and a reason — a row missing any of the three
# is a hard error naming the offending line number. A `pass` consumer
# that STILL has a waiver row fails the gate as a stale waiver (task 2
# wires that check up against the tool's actual JSON output).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

WAIVER_FILE="$SCRIPT_DIR/consumer-gate-waivers.txt"
WAIVER_SLUGS=()
WAIVER_BLOCKS=()
WAIVER_REASONS=()

trim() {
    printf '%s' "$1" | sed -E 's/^[[:space:]]+//; s/[[:space:]]+$//'
}

# Parse and validate WAIVER_FILE into WAIVER_SLUGS/WAIVER_BLOCKS/
# WAIVER_REASONS. `#` comments and blank lines are ignored. A row that
# splits into fewer than three non-empty fields (slug | owning-block-id |
# reason) is a hard error naming WAIVER_FILE and the offending line
# number — a waiver with no owning block is how debt becomes permanent.
#
# Slug validity against the tool's discovered consumer list is NOT
# checked here — this parser is pure text handling with no dependency on
# `mev check-consumers` having run yet (task 2 cross-checks waived slugs
# against the tool's actual JSON output once the invocation exists).
parse_waivers() {
    WAIVER_SLUGS=()
    WAIVER_BLOCKS=()
    WAIVER_REASONS=()
    [ -f "$WAIVER_FILE" ] || return 0

    local lineno=0 rawline trimmed f1 f2 f3
    while IFS= read -r rawline || [ -n "$rawline" ]; do
        lineno=$((lineno + 1))
        trimmed="$(trim "$rawline")"
        [ -z "$trimmed" ] && continue
        case "$trimmed" in
            \#*) continue ;;
        esac

        IFS='|' read -r f1 f2 f3 <<< "$trimmed"
        f1="$(trim "${f1:-}")"
        f2="$(trim "${f2:-}")"
        f3="$(trim "${f3:-}")"

        if [ -z "$f1" ] || [ -z "$f2" ] || [ -z "$f3" ]; then
            echo "check_consumers: malformed waiver row at $WAIVER_FILE:$lineno — need 3 fields: slug | owning-block-id | reason" >&2
            exit 1
        fi

        WAIVER_SLUGS+=("$f1")
        WAIVER_BLOCKS+=("$f2")
        WAIVER_REASONS+=("$f3")
    done < "$WAIVER_FILE"
}

# Sets WAIVER_OWNER to $1's owning block id and returns 0 if $1 has a
# waiver row; returns 1 (WAIVER_OWNER cleared) otherwise.
WAIVER_OWNER=""
lookup_waiver() {
    local slug="$1" i
    WAIVER_OWNER=""
    for i in "${!WAIVER_SLUGS[@]}"; do
        if [ "${WAIVER_SLUGS[$i]}" = "$slug" ]; then
            WAIVER_OWNER="${WAIVER_BLOCKS[$i]}"
            return 0
        fi
    done
    return 1
}

# ---------------------------------------------------------------------------
# TODO (MV.18.A task 2): invoke `cargo run --release --quiet --
# check-consumers --json` from $REPO_ROOT (never a `mev` resolved from
# PATH — overridable via MEV_CHECK_CONSUMERS_CMD for the fixture suite),
# parse the {slug, outcome} array, adjudicate against the waivers parsed
# above, print the per-consumer summary, and decide the exit code:
# non-zero iff an unwaived consumer is `broken`, or a waiver names a
# consumer whose outcome is `pass` (a stale waiver). `skipped_dirty`,
# `lockfile_stale` and `not_evaluable` always exit 0.
# ---------------------------------------------------------------------------
main() {
    parse_waivers
}

main "$@"
