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
# that STILL has a waiver row fails the gate as a stale waiver.
#
# Invocation + adjudication (task 2, this file): the wrapper runs mev
# from the SOURCE tree — `cargo run --release --quiet -- check-consumers
# --json` from $REPO_ROOT — never a `mev` resolved from PATH. The gate's
# whole purpose is "does the mev in THIS working tree break its
# consumers"; an installed binary answers a different question about a
# different revision. `cargo build --release` is already a gating check,
# so the release artifact is paid for either way.
# MEV_CHECK_CONSUMERS_CMD overrides the invocation (a shell command
# string, eval'd from $REPO_ROOT) so the fixture suite can substitute a
# canned JSON emitter without compiling anything.
#
# The tool's JSON is `[{"slug":"...","outcome":{"outcome":"pass"|
# "broken"|"lockfile_stale"|"skipped_dirty"|"not_evaluable", ...}}]` —
# `outcome` is serde-internally-tagged with its own tag key also named
# `outcome` (`#[serde(tag = "outcome", ...)]` on ConsumerOutcome), so the
# per-consumer verdict string sits at `"outcome":"<verdict>"` nested
# inside the `outcome` object; `broken` additionally carries an `errors`
# array, `not_evaluable` a `reason` string. This wrapper does not re-derive
# any of that classification — it only reads the tags the tool already
# assigned.
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
# Invocation: run mev from the source tree, never a `mev` resolved from
# PATH. The default command is the literal `cargo run --release --quiet
# -- check-consumers --json`, always run from $REPO_ROOT so `cargo`
# resolves this checkout's own Cargo.toml. MEV_CHECK_CONSUMERS_CMD, when
# set, replaces the command entirely (eval'd as a shell command string) —
# the fixture suite's only hook for substituting a canned JSON emitter.
# ---------------------------------------------------------------------------
default_check_consumers_cmd() {
    printf '%s' 'cargo run --release --quiet -- check-consumers --json'
}

invoke_check_consumers() {
    local cmd="${MEV_CHECK_CONSUMERS_CMD:-$(default_check_consumers_cmd)}"
    local out
    if ! out="$(cd "$REPO_ROOT" && eval "$cmd")"; then
        echo "check_consumers: check-consumers invocation failed: $cmd" >&2
        exit 1
    fi
    printf '%s' "$out"
}

# ---------------------------------------------------------------------------
# JSON parsing: split the top-level array into one object substring per
# consumer, then pull out the fields this wrapper cares about with plain
# text handling (no jq, no python — same constraint as the waiver
# parser). The JSON here is machine-generated by mev's own
# `serde_json::to_string`, compact and single-line, so a brace-depth /
# quote-aware scan is enough to find the top-level object boundaries;
# per-object field extraction then reads as flat regexes because none of
# the fields this wrapper reads are themselves object-valued strings.
# ---------------------------------------------------------------------------

# Print each top-level `{...}` object in JSON array $1, one per line (as
# a single-line object string each — no embedded newlines are possible
# since the input is already single-line).
split_json_objects() {
    awk -v s="$1" '
        BEGIN {
            n = length(s)
            depth = 0
            instr = 0
            esc = 0
            start = 0
            for (i = 1; i <= n; i++) {
                c = substr(s, i, 1)
                if (instr) {
                    if (esc) { esc = 0 }
                    else if (c == "\\") { esc = 1 }
                    else if (c == "\"") { instr = 0 }
                    continue
                }
                if (c == "\"") { instr = 1; continue }
                if (c == "{") {
                    if (depth == 0) start = i
                    depth++
                } else if (c == "}") {
                    depth--
                    if (depth == 0) {
                        print substr(s, start, i - start + 1)
                    }
                }
            }
        }
    '
}

# Extract "slug":"<value>" from consumer-object string $1.
json_field_slug() {
    printf '%s' "$1" | grep -oE '"slug":"([^"\\]|\\.)*"' | head -n1 \
        | sed -E 's/^"slug":"//; s/"$//'
}

# Extract the outcome tag ("pass" | "broken" | "lockfile_stale" |
# "skipped_dirty" | "not_evaluable") from consumer-object string $1. The
# outer field is also named `outcome` but its value is an object (starts
# with `{`), never a bare string, so this pattern only ever matches the
# inner serde tag.
json_field_outcome() {
    printf '%s' "$1" \
        | grep -oE '"outcome":"(pass|broken|lockfile_stale|skipped_dirty|not_evaluable)"' \
        | head -n1 \
        | sed -E 's/^"outcome":"//; s/"$//'
}

# Extract each element of a "errors":[...] array from consumer-object
# string $1, one per line. Empty output if there is no such array (every
# outcome but `broken`).
json_field_errors() {
    local arr
    arr="$(printf '%s' "$1" | grep -oE '"errors":\[[^]]*\]' | head -n1)"
    [ -n "$arr" ] || return 0
    printf '%s' "$arr" | grep -oE '"([^"\\]|\\.)*"' | tail -n +2 \
        | sed -E 's/^"//; s/"$//'
}

# Extract "reason":"<value>" from consumer-object string $1 (present
# only for `not_evaluable`). Empty if absent.
json_field_reason() {
    printf '%s' "$1" | grep -oE '"reason":"([^"\\]|\\.)*"' | head -n1 \
        | sed -E 's/^"reason":"//; s/"$//'
}

# ---------------------------------------------------------------------------
# Adjudication: exit non-zero iff (a) some consumer is `broken` and has
# no waiver row, or (b) some waiver row names a consumer whose outcome is
# `pass` (a stale waiver). `skipped_dirty`, `lockfile_stale` and
# `not_evaluable` are reported and always exit 0 — they are bookkeeping
# about someone else's repo, not evidence mev broke anything.
# ---------------------------------------------------------------------------
main() {
    parse_waivers

    local raw
    raw="$(invoke_check_consumers)"

    local objects
    objects="$(split_json_objects "$raw")"

    local gate_failed=0
    local seen_slugs=()
    local obj slug outcome owner note errors_str reason

    if [ -n "$objects" ]; then
        while IFS= read -r obj; do
            [ -n "$obj" ] || continue
            slug="$(json_field_slug "$obj")"
            outcome="$(json_field_outcome "$obj")"
            seen_slugs+=("$slug")

            owner=""
            if lookup_waiver "$slug"; then
                owner="$WAIVER_OWNER"
            fi

            note=""
            case "$outcome" in
                pass)
                    if [ -n "$owner" ]; then
                        note="stale waiver (owned by $owner) — consumer now passes; delete this waiver row"
                        gate_failed=1
                    fi
                    ;;
                broken)
                    if [ -n "$owner" ]; then
                        note="waived by $owner"
                    else
                        gate_failed=1
                    fi
                    ;;
                lockfile_stale|skipped_dirty|not_evaluable)
                    if [ -n "$owner" ]; then
                        note="waived by $owner"
                    fi
                    ;;
                *)
                    echo "check_consumers: unrecognized outcome for consumer '$slug' in tool output" >&2
                    exit 1
                    ;;
            esac

            if [ -n "$note" ]; then
                echo "== $slug: $outcome ($note) =="
            else
                echo "== $slug: $outcome =="
            fi

            if [ "$outcome" = "broken" ]; then
                while IFS= read -r errors_str; do
                    [ -n "$errors_str" ] || continue
                    echo "  $errors_str"
                done < <(json_field_errors "$obj")
            elif [ "$outcome" = "not_evaluable" ]; then
                reason="$(json_field_reason "$obj")"
                [ -n "$reason" ] && echo "  $reason"
            fi
        done <<< "$objects"
    fi

    # A waiver naming a consumer the tool never reported at all is not
    # adjudicable — surface it rather than silently ignoring it, though
    # it does not by itself fail the gate (the tool's discovery, not this
    # wrapper, owns the consumer set).
    local w
    for w in "${WAIVER_SLUGS[@]+"${WAIVER_SLUGS[@]}"}"; do
        local known=0 s
        for s in "${seen_slugs[@]+"${seen_slugs[@]}"}"; do
            [ "$s" = "$w" ] && known=1 && break
        done
        if [ "$known" -eq 0 ]; then
            echo "check_consumers: waiver names '$w', which the tool did not report — check the slug" >&2
        fi
    done

    if [ "$gate_failed" -eq 1 ]; then
        exit 1
    fi
    exit 0
}

main "$@"
