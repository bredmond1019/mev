#!/usr/bin/env bash
#
# test_check_consumers.sh — fixture-driven regression suite for
# scripts/check_consumers.sh (MV.18.A task 3).
#
# check_consumers.sh's only job is to invoke `mev check-consumers --json`
# from the source tree, apply the waiver list, and decide the exit code —
# it never re-derives discovery, spawning, lockfile hashing or
# classification (those stay in src/consumers/mod.rs). So this suite does
# NOT rebuild a fake brain.toml / consumer repo tree the way okf-core's
# does; instead it drives the wrapper through `MEV_CHECK_CONSUMERS_CMD`,
# which the wrapper reads to replace its `check-consumers` invocation
# entirely, pointed at a canned JSON emitter (a plain `cat` of a fixture
# file) that reproduces the exact `[{slug, outcome:{outcome, ...}}]` shape
# `mev check-consumers --json` emits (src/consumers/mod.rs's
# `ConsumerOutcome`, internally tagged on its own `outcome` key).
#
# `cargo` and `git` are still shimmed onto a PATH prepended ahead of the
# real ones, in a mktemp -d, so:
#   - the ONE case that exercises the wrapper's DEFAULT invocation (no
#     MEV_CHECK_CONSUMERS_CMD override) proves the wrapper actually runs
#     `cargo run --release --quiet -- check-consumers --json` from the
#     source tree rather than compiling anything for real — the cargo
#     shim intercepts that exact invocation and answers with canned JSON;
#   - every other case, and the suite as a whole, never spawns a real
#     cargo build and never touches a repo outside $WORK.
# A `mev` binary is also shimmed onto PATH and asserted to have logged
# ZERO invocations across the entire suite — the proof the wrapper never
# resolves a `mev` from PATH, only ever `cargo run` against this checkout.
#
#   bash scripts/test_check_consumers.sh
#
# Exit status 0 = every case passed; non-zero = at least one failure.
#
set -uo pipefail

fail=0
pass_count=0
fail_count=0
check() { # check <description> <result: 0=pass>
    if [ "$2" -eq 0 ]; then
        printf 'ok   %s\n' "$1"
        pass_count=$((pass_count + 1))
    else
        printf 'FAIL %s\n' "$1"
        fail=1
        fail_count=$((fail_count + 1))
    fi
}

SELF_DIR="$(cd "$(dirname "$0")" && pwd)"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# ---------------------------------------------------------------------------
# Fixture copy of the wrapper under test, in its own scripts/ dir so
# WAIVER_FILE (which check_consumers.sh resolves as $SCRIPT_DIR-relative)
# lands on a per-suite-run waiver file, never the repo's real one.
# ---------------------------------------------------------------------------
GATE_DIR="$WORK/scripts"
mkdir -p "$GATE_DIR"
cp "$SELF_DIR/check_consumers.sh" "$GATE_DIR/check_consumers.sh"
chmod +x "$GATE_DIR/check_consumers.sh"
WAIVER_FILE="$GATE_DIR/consumer-gate-waivers.txt"
: > "$WAIVER_FILE"

# ---------------------------------------------------------------------------
# Shim bin dir, prepended onto PATH ahead of the real cargo/git/mev.
# ---------------------------------------------------------------------------
BIN="$WORK/bin"
mkdir -p "$BIN"

CARGO_LOG="$WORK/cargo-calls.log"       # cleared by reset_fixtures
CARGO_LOG_ALL="$WORK/cargo-calls-all.log"  # never cleared — cumulative proof
GIT_LOG_ALL="$WORK/git-calls-all.log"
MEV_LOG_ALL="$WORK/mev-calls-all.log"   # never cleared — must stay empty
: > "$CARGO_LOG"
: > "$CARGO_LOG_ALL"
: > "$GIT_LOG_ALL"
: > "$MEV_LOG_ALL"

cat > "$BIN/cargo" <<SH
#!/usr/bin/env bash
echo "\$@" >> "$CARGO_LOG"
echo "\$@" >> "$CARGO_LOG_ALL"
if [ -n "\${CANNED_JSON_FILE:-}" ] && [ -f "\${CANNED_JSON_FILE:-}" ]; then
    cat "\$CANNED_JSON_FILE"
    exit 0
fi
echo '[]'
exit 0
SH
chmod +x "$BIN/cargo"

cat > "$BIN/git" <<SH
#!/usr/bin/env bash
echo "\$@" >> "$GIT_LOG_ALL"
exit 0
SH
chmod +x "$BIN/git"

cat > "$BIN/mev" <<SH
#!/usr/bin/env bash
echo "\$@" >> "$MEV_LOG_ALL"
echo '[]'
exit 1
SH
chmod +x "$BIN/mev"

TEST_PATH="$BIN:$PATH"

# ---------------------------------------------------------------------------
# Canned JSON fixtures reproducing mev check-consumers --json's exact shape.
# ---------------------------------------------------------------------------
FIX="$WORK/fixtures"
mkdir -p "$FIX"

printf '%s' '[{"slug":"bastion","outcome":{"outcome":"pass"}},{"slug":"engine-rs","outcome":{"outcome":"pass"}}]' \
    > "$FIX/all_pass.json"

printf '%s' '[{"slug":"bastion","outcome":{"outcome":"broken","errors":["E0063 at src/serve/handlers/board.rs:660:9","E0308 at src/serve/handlers/block_graph.rs:414:22"]}},{"slug":"engine-rs","outcome":{"outcome":"pass"}}]' \
    > "$FIX/one_broken.json"

printf '%s' '[{"slug":"mev","outcome":{"outcome":"pass"}}]' \
    > "$FIX/pass_one.json"

printf '%s' '[{"slug":"bastion","outcome":{"outcome":"skipped_dirty"}}]' \
    > "$FIX/skipped_dirty_alone.json"

printf '%s' '[{"slug":"engine-rs","outcome":{"outcome":"lockfile_stale"}}]' \
    > "$FIX/lockfile_stale_alone.json"

printf '%s' '[{"slug":"mev","outcome":{"outcome":"not_evaluable","reason":"unrecognized failure (exit 1)"}}]' \
    > "$FIX/not_evaluable_alone.json"

# ---------------------------------------------------------------------------
# Fixture helpers
# ---------------------------------------------------------------------------
reset_fixtures() {
    : > "$WAIVER_FILE"
    : > "$CARGO_LOG"
}

set_waiver_file() { # set_waiver_file <content>
    printf '%s' "$1" > "$WAIVER_FILE"
}

# run_gate <fixture-json-path> -- runs the wrapper with
# MEV_CHECK_CONSUMERS_CMD overridden to `cat` the fixture file, sets OUT/RC.
run_gate() {
    OUT="$(PATH="$TEST_PATH" MEV_CHECK_CONSUMERS_CMD="cat '$1'" "$GATE_DIR/check_consumers.sh" 2>&1)"
    RC=$?
}

# run_gate_default -- runs the wrapper with NO override, so it takes the
# default `cargo run --release --quiet -- check-consumers --json` path;
# the shimmed cargo answers with $CANNED_JSON_FILE's content.
run_gate_default() {
    OUT="$(PATH="$TEST_PATH" CANNED_JSON_FILE="$1" "$GATE_DIR/check_consumers.sh" 2>&1)"
    RC=$?
}

# ---------------------------------------------------------------------------
# Case: default invocation (no MEV_CHECK_CONSUMERS_CMD) — proves the
# wrapper runs mev from SOURCE via `cargo run`, never a `mev` on PATH.
# ---------------------------------------------------------------------------
reset_fixtures
run_gate_default "$FIX/all_pass.json"
check "default invocation exits 0 against an all-pass canned response" \
    "$( [ "$RC" -eq 0 ] && echo 0 || echo 1 )"
check "default invocation used 'cargo run --release --quiet -- check-consumers --json'" \
    "$(grep -q -- '--release --quiet -- check-consumers --json' "$CARGO_LOG" && echo 0 || echo 1)"
check "mev on PATH was never invoked during the default invocation" \
    "$( [ ! -s "$MEV_LOG_ALL" ] && echo 0 || echo 1 )"

# ---------------------------------------------------------------------------
# Case: all consumers pass, no waivers -> exit 0.
# ---------------------------------------------------------------------------
reset_fixtures
run_gate "$FIX/all_pass.json"
check "all pass, no waivers -> exit 0" \
    "$( [ "$RC" -eq 0 ] && echo 0 || echo 1 )"

# ---------------------------------------------------------------------------
# Case: one broken, unwaived -> non-zero; names the consumer and both
# error sites.
# ---------------------------------------------------------------------------
reset_fixtures
run_gate "$FIX/one_broken.json"
check "one broken, unwaived -> exits non-zero" \
    "$( [ "$RC" -ne 0 ] && echo 0 || echo 1 )"
check "broken report names the consumer and both error sites" \
    "$(printf '%s' "$OUT" | grep -q 'bastion: broken' \
        && printf '%s' "$OUT" | grep -q 'board.rs:660:9' \
        && printf '%s' "$OUT" | grep -q 'block_graph.rs:414:22' \
        && echo 0 || echo 1)"

# ---------------------------------------------------------------------------
# Case: the SAME broken consumer WITH a waiver row -> exit 0; summary
# names the owning block.
# ---------------------------------------------------------------------------
reset_fixtures
set_waiver_file 'bastion | OP.fix-bastion | bastion known broken pending OP.fix-bastion'
run_gate "$FIX/one_broken.json"
check "broken + waived -> exit 0" \
    "$( [ "$RC" -eq 0 ] && echo 0 || echo 1 )"
check "broken + waived summary names the owning block" \
    "$(printf '%s' "$OUT" | grep -q 'waived by OP.fix-bastion' && echo 0 || echo 1)"

# ---------------------------------------------------------------------------
# Case: stale waiver (waiver names a consumer now passing) -> non-zero;
# the shown-failing pair — delete the row and the same fixture exits 0.
# ---------------------------------------------------------------------------
reset_fixtures
set_waiver_file 'mev | OP.old-fix | mev used to be broken'
run_gate "$FIX/pass_one.json"
check "stale waiver: pass + waived exits non-zero" \
    "$( [ "$RC" -ne 0 ] && printf '%s' "$OUT" | grep -q 'stale waiver' && echo 0 || echo 1 )"

set_waiver_file ''
run_gate "$FIX/pass_one.json"
check "SHOWN-FAILING: deleting the stale waiver row makes the same fixture exit 0" \
    "$( [ "$RC" -eq 0 ] && echo 0 || echo 1 )"

# ---------------------------------------------------------------------------
# Case: skipped_dirty / lockfile_stale / not_evaluable each exit 0 on
# their own — three SEPARATE cases, never conflated with broken.
# ---------------------------------------------------------------------------
reset_fixtures
run_gate "$FIX/skipped_dirty_alone.json"
check "skipped_dirty alone -> exit 0" \
    "$( [ "$RC" -eq 0 ] && echo 0 || echo 1 )"

reset_fixtures
run_gate "$FIX/lockfile_stale_alone.json"
check "lockfile_stale alone -> exit 0" \
    "$( [ "$RC" -eq 0 ] && echo 0 || echo 1 )"

reset_fixtures
run_gate "$FIX/not_evaluable_alone.json"
check "not_evaluable alone -> exit 0" \
    "$( [ "$RC" -eq 0 ] && echo 0 || echo 1 )"

# ---------------------------------------------------------------------------
# Case: a waiver row missing each of the three fields in turn -> non-zero,
# naming the offending line number.
# ---------------------------------------------------------------------------
reset_fixtures
set_waiver_file ' | OP.some-block | missing-slug'
run_gate "$FIX/all_pass.json"
check "waiver row missing slug -> hard error naming line 1" \
    "$( [ "$RC" -ne 0 ] && printf '%s' "$OUT" | grep -q 'malformed waiver row' \
        && printf '%s' "$OUT" | grep -q ':1 ' && echo 0 || echo 1 )"

reset_fixtures
set_waiver_file 'bastion |  | missing-owning-block'
run_gate "$FIX/all_pass.json"
check "waiver row missing owning-block -> hard error naming line 1" \
    "$( [ "$RC" -ne 0 ] && printf '%s' "$OUT" | grep -q 'malformed waiver row' \
        && printf '%s' "$OUT" | grep -q ':1 ' && echo 0 || echo 1 )"

reset_fixtures
set_waiver_file 'bastion | OP.some-block | '
run_gate "$FIX/all_pass.json"
check "waiver row missing reason -> hard error naming line 1" \
    "$( [ "$RC" -ne 0 ] && printf '%s' "$OUT" | grep -q 'malformed waiver row' \
        && printf '%s' "$OUT" | grep -q ':1 ' && echo 0 || echo 1 )"

# ---------------------------------------------------------------------------
# Case: `#` comments and blank lines in the waiver file are ignored.
# ---------------------------------------------------------------------------
reset_fixtures
set_waiver_file $'# header comment\n\nbastion | OP.fix-bastion | bastion known broken\n'
run_gate "$FIX/one_broken.json"
check "comments and blank lines ignored; the real waiver row still applies -> exit 0" \
    "$( [ "$RC" -eq 0 ] && printf '%s' "$OUT" | grep -q 'waived by OP.fix-bastion' && echo 0 || echo 1 )"

# ---------------------------------------------------------------------------
# Syntax sanity, in-suite (also asserted externally by task validation).
# ---------------------------------------------------------------------------
check "check_consumers.sh passes bash -n" \
    "$(bash -n "$GATE_DIR/check_consumers.sh" 2>/dev/null && echo 0 || echo 1)"

# ---------------------------------------------------------------------------
# Isolation + no-real-build guards: across the WHOLE suite, no cargo
# invocation ever named `nextest` (i.e. no real consumer test target was
# ever compiled), and `mev` on PATH was never invoked even once.
# ---------------------------------------------------------------------------
check "no real consumer build was ever spawned (no nextest invocation logged)" \
    "$( ! grep -q 'nextest' "$CARGO_LOG_ALL" && echo 0 || echo 1 )"
check "mev on PATH recorded ZERO invocations across the entire suite" \
    "$( [ ! -s "$MEV_LOG_ALL" ] && echo 0 || echo 1 )"

echo
echo "== $pass_count passed, $fail_count failed =="
exit "$fail"
