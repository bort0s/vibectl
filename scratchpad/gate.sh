#!/usr/bin/env bash
#
# gate.sh — the local gate, with a channel that has been measured.
#
# WHY THIS FILE EXISTS. The gate was `cargo test --workspace | grep -c FAILED`,
# and that returns **0 when everything passes and 0 when nothing compiles**. It
# reported "0 failures" on a tree where `monitor_writer` did not build. Every
# "0 failures" claimed from that channel is of unknown standing.
#
# It is the project's own rule applied to everything except itself: an empty
# result without a positive control is a skipped test wearing a green check, and
# a channel between harness and subject has to be measured with a KNOWN INPUT
# before it is trusted. This channel never was.
#
# It is also a third category the base rate did not cover. "Suspect the tool
# first" contrasts the tool with the code under test. This is neither: it is the
# HARNESS REPORTING ON ITSELF, and it had no control at all. The category opens
# with a negative result.
#
# WHAT IS DIFFERENT NOW.
#
#   1. Every step is judged by its EXIT CODE, never by grepping its output.
#   2. `set -o pipefail`, so a pipeline's status is not just its last command's.
#      There are no pipes in the judgements below, and pipefail is set anyway —
#      the next person to add one should not have to know this.
#   3. `--sabotage` runs the gate against two deliberately broken trees, a
#      FAILING test and a NON-COMPILING one, and requires red from both. One
#      colour is not enough: the failure this file exists for was a
#      non-compiling tree reading as green, which a failing-test check would
#      never have caught.
#   4. Every number printed says what it is derived from, because "29 test
#      binaries" came through the same channel as the zero did.
#
# usage: bash scratchpad/gate.sh            # run the gate
#        bash scratchpad/gate.sh --sabotage # prove the gate can go red, twice
set -u
set -o pipefail

cd "$(dirname "$0")/.." || exit 2

step() {
  local name="$1"; shift
  "$@" >/dev/null 2>&1
  local code=$?
  printf '  %-28s exit=%d\n' "$name" "$code"
  return $code
}

run_gate() {
  local failed=0
  step "fmt --check"        cargo fmt --all -- --check                              || failed=1
  step "clippy -D warnings" cargo clippy --workspace --all-targets -- -D warnings   || failed=1
  step "test --workspace"   cargo test --workspace                                  || failed=1
  step "MSRV 1.85 --locked" cargo +1.85.0 check --locked --workspace                || failed=1
  return $failed
}

derive_numbers() {
  local out
  out="$(mktemp -t gate.XXXXXX)"
  cargo test --workspace >"$out" 2>&1
  local code=$?
  # DERIVATIONS, stated because a number is only as good as where it came from:
  #   binaries  = lines matching '^test result:' — cargo prints one per test
  #               binary it RAN. It is 0 when nothing compiled, which is the
  #               whole point; the exit code above is what decides pass/fail.
  #   passed    = the sum of the "N passed" fields on those lines.
  #   failed    = the sum of the "N failed" fields.
  local binaries passed failed
  binaries=$(grep -c '^test result:' "$out")
  passed=$(awk '/^test result:/ { for (i=1;i<=NF;i++) if ($(i+1)=="passed;") s+=$i } END { print s+0 }' "$out")
  failed=$(awk '/^test result:/ { for (i=1;i<=NF;i++) if ($(i+1)=="failed;") s+=$i } END { print s+0 }' "$out")
  printf '  test binaries that RAN (count of "^test result:" lines): %s\n' "${binaries:-0}"
  printf '  tests passed (sum of "N passed" on those lines):         %s\n' "${passed:-0}"
  printf '  tests failed (sum of "N failed" on those lines):         %s\n' "${failed:-0}"
  printf '  cargo test exit code (the only thing that decides):      %s\n' "$code"
  rm -f "$out"
  return $code
}

sabotage() {
  local victim="crates/vibe-core/tests/atomic_replace.rs"
  local backup
  backup="$(mktemp -t sabotage.XXXXXX)"
  cp "$victim" "$backup"
  local overall=0

  echo "SABOTAGE 1 — a test that FAILS (compiles, asserts something false)"
  printf '\n#[test]\nfn deliberate_failure() { assert_eq!(1, 2, "sabotage"); }\n' >>"$victim"
  if run_gate; then
    echo "  *** THE GATE STAYED GREEN ON A FAILING TEST ***"
    overall=1
  else
    echo "  red, as required"
  fi
  cp "$backup" "$victim"

  echo
  echo "SABOTAGE 2 — a tree that DOES NOT COMPILE (the case that got through)"
  printf '\nfn deliberate_type_error() { let _: u32 = "not a number"; }\n' >>"$victim"
  if run_gate; then
    echo "  *** THE GATE STAYED GREEN ON A NON-COMPILING TREE ***"
    overall=1
  else
    echo "  red, as required"
  fi
  cp "$backup" "$victim"
  rm -f "$backup"

  echo
  if [ "$overall" -eq 0 ]; then
    echo "both sabotages produced red; the gate's channel is measured"
  else
    echo "THE GATE IS NOT TRUSTWORTHY — see above"
  fi
  return $overall
}

if [ "${1:-}" = "--sabotage" ]; then
  sabotage
  exit $?
fi

echo "gate on $(git rev-parse --abbrev-ref HEAD) @ $(git rev-parse --short HEAD)"
run_gate
gate_code=$?
echo
derive_numbers >/dev/null 2>&1 || true
derive_numbers
echo
if [ "$gate_code" -eq 0 ]; then
  echo "GREEN — every step exited 0"
else
  echo "RED — at least one step did not exit 0"
fi
exit $gate_code
