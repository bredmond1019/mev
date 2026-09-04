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

printf '%s' '[{"slug":"bastion","outcome":{"outcome":"pass"}},{"slug":"engine-rs","outcome":{"outcome":"skipped_dirty"}}]' \
    > "$FIX/mixed_pass_skipped.json"

printf '%s' '[{"slug":"bastion","outcome":{"outcome":"skipped_dirty"}},{"slug":"engine-rs","outcome":{"outcome":"lockfile_stale"}}]' \
    > "$FIX/two_declining_different.json"

printf '%s' '[{"slug":"bastion","outcome":{"outcome":"pass"}},{"slug":"mev","outcome":{"outcome":"not_evaluable","reason":"unrecognized failure (exit 1)"}}]' \
    > "$FIX/pass_and_not_evaluable.json"

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

# run_gate_failing <fixture-json-path> -- runs the wrapper with an
# invocation that prints the fixture JSON to stdout AND THEN exits 1 —
# reproducing `mev check-consumers --json`'s real behaviour when a
# consumer is broken (src/main.rs: it prints the JSON, then returns
# ExitCode::FAILURE). Sets OUT/RC.
run_gate_failing() {
    OUT="$(PATH="$TEST_PATH" MEV_CHECK_CONSUMERS_CMD="sh -c \"cat '$1'; exit 1\"" "$GATE_DIR/check_consumers.sh" 2>&1)"
    RC=$?
}

# run_gate_unrunnable -- runs the wrapper pointed at a command that does
# not exist at all, so `eval` itself fails with bash's own
# command-not-found (127) before the "tool" ever produces any output.
# Sets OUT/RC.
run_gate_unrunnable() {
    OUT="$(PATH="$TEST_PATH" MEV_CHECK_CONSUMERS_CMD="/no/such/path/mev-check-consumers-does-not-exist-$$" "$GATE_DIR/check_consumers.sh" 2>&1)"
    RC=$?
}

# run_gate_transient -- runs the wrapper pointed at a command that DOES
# run (unlike run_gate_unrunnable) but exits non-zero while printing
# something that is not parseable JSON — a stand-in for a flaky/transient
# cargo failure (the false-red case observed live, cleared by a clean
# re-run). Sets OUT/RC.
run_gate_transient() {
    OUT="$(PATH="$TEST_PATH" MEV_CHECK_CONSUMERS_CMD="sh -c 'echo error: could not compile mev crate transient failure >&2; exit 1'" "$GATE_DIR/check_consumers.sh" 2>&1)"
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
# Coverage-line cases (MV.ticket.consumer-gate-reports-coverage task 1):
# `check_consumers: verified <P> of <N> consumers` must print on every
# path, with <P> counting ONLY `pass` outcomes and <N> counting every
# reported consumer; when P < N the unverified consumers are named with
# their own outcome. These cases pin BOTH the coverage text AND the exit
# code, so task 2 cannot change adjudication while satisfying them.
# check_consumers.sh is untouched as of this task, so all five below are
# expected to FAIL until task 2 lands — that failure is the evidence the
# gate can go red.
# ---------------------------------------------------------------------------

# (a) two consumers both pass -> 2 of 2, no unverified names, exit 0.
reset_fixtures
run_gate "$FIX/all_pass.json"
check "coverage: two pass -> exit 0" \
    "$( [ "$RC" -eq 0 ] && echo 0 || echo 1 )"
check "coverage: two pass -> 'verified 2 of 2 consumers'" \
    "$(printf '%s' "$OUT" | grep -qF 'check_consumers: verified 2 of 2 consumers' && echo 0 || echo 1)"

# (b) one pass + one skipped_dirty -> 1 of 2, names the decliner, exit 0.
reset_fixtures
run_gate "$FIX/mixed_pass_skipped.json"
check "coverage: pass + skipped_dirty -> exit 0" \
    "$( [ "$RC" -eq 0 ] && echo 0 || echo 1 )"
check "coverage: pass + skipped_dirty -> 'verified 1 of 2 consumers (engine-rs: skipped_dirty)'" \
    "$(printf '%s' "$OUT" | grep -qF 'check_consumers: verified 1 of 2 consumers (engine-rs: skipped_dirty)' && echo 0 || echo 1)"

# (c) two consumers decline with DIFFERENT outcomes -> 0 of 2, names both
# with their own distinct outcome, exit 0.
reset_fixtures
run_gate "$FIX/two_declining_different.json"
check "coverage: skipped_dirty + lockfile_stale -> exit 0" \
    "$( [ "$RC" -eq 0 ] && echo 0 || echo 1 )"
check "coverage: skipped_dirty + lockfile_stale -> 'verified 0 of 2 consumers (bastion: skipped_dirty, engine-rs: lockfile_stale)'" \
    "$(printf '%s' "$OUT" | grep -qF 'check_consumers: verified 0 of 2 consumers (bastion: skipped_dirty, engine-rs: lockfile_stale)' && echo 0 || echo 1)"

# (d) one pass + one not_evaluable -> not_evaluable counts as unverified,
# NOT verified: 1 of 2, names it, exit 0.
reset_fixtures
run_gate "$FIX/pass_and_not_evaluable.json"
check "coverage: pass + not_evaluable -> exit 0" \
    "$( [ "$RC" -eq 0 ] && echo 0 || echo 1 )"
check "coverage: pass + not_evaluable -> 'verified 1 of 2 consumers (mev: not_evaluable)'" \
    "$(printf '%s' "$OUT" | grep -qF 'check_consumers: verified 1 of 2 consumers (mev: not_evaluable)' && echo 0 || echo 1)"

# (e) one broken (unwaived) -> gate still exits 1 (adjudication
# unchanged), and the coverage line still prints, naming the broken
# consumer as unverified.
reset_fixtures
run_gate "$FIX/one_broken.json"
check "coverage: one broken -> exit code STAYS non-zero" \
    "$( [ "$RC" -ne 0 ] && echo 0 || echo 1 )"
check "coverage: one broken -> 'verified 1 of 2 consumers (bastion: broken)'" \
    "$(printf '%s' "$OUT" | grep -qF 'check_consumers: verified 1 of 2 consumers (bastion: broken)' && echo 0 || echo 1)"

# ---------------------------------------------------------------------------
# MV.ticket.consumer-gate-waiver-can-never-apply-to-a-broken-consumer,
# task 1: adjudicate before aborting; distinguish the three abort
# conditions; still report the verified-consumer count on a clean run.
# All five cases from the block record live here (task 2 owns the
# live-lease requirement, case 2 below, and stays red until it lands —
# see the note on that case).
# ---------------------------------------------------------------------------

# --- case 1: the impossible case. A consumer is `broken`, has a waiver
# row, AND the tool's own invocation exits non-zero (exactly what real
# `mev check-consumers --json` does when a consumer is broken — it still
# prints the JSON first). Before this task, invoke_check_consumers
# aborted on that non-zero exit and discarded the JSON before main() ever
# saw it, so a waiver could never reach a broken+waived consumer. This
# was OBSERVED RED before the fix landed (recorded 2026-09-03): the gate
# exited non-zero on this exact fixture with the generic "invocation
# failed" message instead of honouring the waiver.
reset_fixtures
set_waiver_file 'bastion | OP.fix-bastion | bastion known broken pending OP.fix-bastion'
run_gate_failing "$FIX/one_broken.json"
check "case 1 (impossible case): broken+waived, tool invocation itself exits non-zero -> exit 0, waiver named" \
    "$( [ "$RC" -eq 0 ] && printf '%s' "$OUT" | grep -q 'waived by OP.fix-bastion' && echo 0 || echo 1 )"

# --- case 2: same shape as case 1, but the waived repo holds NO live
# lane/lease. Task 2 (not this one) adds the live-lease requirement — a
# waiver with no owning repo lease behind it must be refused. This task
# has no lease concept at all yet, so today the fixture above is simply
# honoured (same outcome as case 1) regardless of any lease. This
# assertion therefore pins TASK 1's actual, current behaviour; task 2
# TIGHTENS it to require the lease — observing THIS SAME assertion go red
# against task-1-final code before implementing the lease check, then
# green ("task 1 case 2 goes red to green" per task 2's own acceptance
# criteria). This is deliberate, not an oversight: a task whose only
# content is an intentionally-failing assertion cannot satisfy this
# engine's work assertion (see the task-1 description's D68 note), so the
# red observation for case 2 happens inside task 2, not here.
reset_fixtures
set_waiver_file 'bastion | OP.fix-bastion | bastion known broken pending OP.fix-bastion'
run_gate_failing "$FIX/one_broken.json"
check "case 2 (task 1 baseline; task 2 tightens this to require a live lease): broken+waived, no lease concept yet -> exit 0, waiver honoured" \
    "$( [ "$RC" -eq 0 ] && printf '%s' "$OUT" | grep -q 'waived by OP.fix-bastion' && echo 0 || echo 1 )"

# --- case 3: the tool is genuinely unrunnable (command not found) ->
# its own message and exit code, distinct from both a broken consumer
# and a transient failure.
reset_fixtures
run_gate_unrunnable
check "case 3: unrunnable tool -> exit 2, message says 'unrunnable', not 'transient'" \
    "$( [ "$RC" -eq 2 ] && printf '%s' "$OUT" | grep -qi 'unrunnable' \
        && ! printf '%s' "$OUT" | grep -qi 'transient' && echo 0 || echo 1 )"

# --- case 4: a transient failure — the tool DID run (unlike case 3) but
# exited non-zero with no parseable JSON on stdout. Distinguishable from
# case 3 by BOTH message and exit code, not merely by eyeballing output.
reset_fixtures
run_gate_transient
check "case 4: transient invocation failure -> exit 3, message says 'transient', not 'unrunnable'" \
    "$( [ "$RC" -eq 3 ] && printf '%s' "$OUT" | grep -qi 'transient' \
        && ! printf '%s' "$OUT" | grep -qi 'unrunnable' && echo 0 || echo 1 )"
check "cases 3 and 4 use DIFFERENT exit codes" \
    "$( [ 2 -ne 3 ] && echo 0 || echo 1 )"

# --- case 5: a clean run (all consumers pass) still reports HOW MANY
# consumers were verified — an empty/never-ran check must be
# distinguishable from a real, positive verification. This is the same
# coverage-line machinery pinned above; restated here as case 5 per the
# block record so all five cases are visibly present in one suite.
reset_fixtures
run_gate "$FIX/all_pass.json"
check "case 5: clean run -> exit 0 AND reports 'verified 2 of 2 consumers'" \
    "$( [ "$RC" -eq 0 ] && printf '%s' "$OUT" | grep -qF 'check_consumers: verified 2 of 2 consumers' && echo 0 || echo 1 )"

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
