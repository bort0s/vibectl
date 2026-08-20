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
#   3. `--sabotage` runs the gate against deliberately broken trees and requires
#      red from each. One colour is not enough, and neither is one shape: the
#      failure this file exists for was a non-compiling tree reading as green,
#      which a failing-test check would never have caught — and the failure the
#      section below records was unix-only code reading as green, which neither
#      of the first two would have caught either.
#   4. Every number printed says what it is derived from, because "29 test
#      binaries" came through the same channel as the zero did.
#   5. Every run says WHICH BINARIES produced its numbers, and every step says
#      WHAT WAS COMPILED to produce them.
#
# ---------------------------------------------------------------------------
# WHAT THE SIXTH INSTRUMENT FINDING WAS, AND WHAT IT WAS NOT.
#
# CI's `Lint (clippy, ubuntu-latest)` failed on `plan.rs:454` — `needless_return`
# — while this gate had reported "clippy exit=0" for the 48 commits since that
# line landed. The standing hypothesis was toolchain drift: an older local
# clippy, a `clippy.toml`, a `[lints]` difference, missing flags. Each was
# measured. Each was FALSE:
#
#   local   clippy 0.1.97 (8bab26f4f6 2026-07-14) / rustc 1.97.1
#   CI      `toolchain: stable` — the same release
#   config  no clippy.toml, no rust-toolchain.toml, no `#[allow]` in scope;
#           `[workspace.lints]` is inherited by both crates identically
#   flags   CI's exact line, run here unchanged, exited 0
#
# The variable was none of those. It was the **target triple**. `cargo clippy`
# compiles for the HOST, this host is `x86_64-pc-windows-msvc`, and `#[cfg(unix)]`
# blocks are stripped during expansion before clippy runs a single lint over
# them. The defect was inside one. The gate was not running a different clippy;
# it was running the same clippy over a DIFFERENT SUBJECT, and reporting the
# number as though the subject were the repository.
#
# So the question "which clippy?" would not have found this. The answer was "the
# same one". The question that finds it is WHAT DID IT COMPILE — which is the
# original `grep -c FAILED` error one level up: a number is only as good as
# where it came from, and where it came from includes the subject, not only the
# instrument.
#
# Four such blocks had never been linted here:
#   plan.rs:442  agents_store.rs:322  atomic_replace.rs:427  ignore_state_git.rs:463
#
# THE REPAIR: clippy runs once per OS family, and `cargo test` says out loud
# that it cannot.
# ---------------------------------------------------------------------------
#
# usage: bash scratchpad/gate.sh            # run the gate
#        bash scratchpad/gate.sh --sabotage # prove the gate can go red, 3 ways
set -u
set -o pipefail

cd "$(dirname "$0")/.." || exit 2

# The location is an assumption, and an instrument run where its assumptions do
# not hold reports red for the wrong reason. That happened the first time this
# was invoked from a copy outside the repo: every step failed and the output
# read like a broken tree. So the assumption is checked, and its failure says
# which one it was.
if [ ! -f Cargo.toml ] || ! grep -q '\[workspace\]' Cargo.toml; then
  echo "gate.sh is not in <workspace>/scratchpad/ — cwd is $(pwd), which has no" >&2
  echo "workspace Cargo.toml. Nothing below would be a measurement of this repo." >&2
  exit 2
fi

# CI's clippy job is a three-OS matrix. Each leg below is the command CI runs,
# aimed at one of those OS families, because a `#[cfg]` the host strips is a
# lint that cannot fire.
#
# The HOST leg deliberately carries NO `--target`. That is not an oversight and
# not cosmetic: without `--target`, RUSTFLAGS reaches build scripts and
# proc-macro dependencies, and ci.yml sets `RUSTFLAGS: -D warnings` at job level
# specifically to catch plain rustc warnings from those. Passing `--target`
# splits host from target artifacts and stops RUSTFLAGS applying to the host
# half. The host leg therefore reproduces CI exactly, and the cross legs are
# strictly about lints in OUR code — which is the hole they exist to close.
#
# `x86_64-apple-darwin`, not `aarch64-apple-darwin`, and the reason is measured
# rather than assumed: blake3 1.8.6's build script compiles NEON C for the
# aarch64 target and there is no cross `cc` here, so that leg cannot run on this
# machine at all. It is a complete stand-in regardless — grepping crates/ for
# `target_arch`, `target_pointer_width` and `target_endian` returns nothing, so
# no `#[cfg]` site in this tree tells the two apart, and macOS reaches every
# block in it through `cfg(unix)`.
#
# Format is `label:triple`, empty triple meaning "no --target".
CLIPPY_LEGS="host: linux:x86_64-unknown-linux-gnu macos:x86_64-apple-darwin"

# CI's msrv job runs on ubuntu-latest, so `cargo check` there compiles the
# `cfg(unix)` half on 1.85. Checking only the host would leave this step with
# exactly the hole clippy had, so it is aimed at linux too.
MSRV_TARGET="x86_64-unknown-linux-gnu"

# WHICH BINARY. `cargo-clippy` on PATH is a rustup SHIM — one dispatcher in
# ~/.cargo/bin that picks a toolchain at call time. `which cargo-clippy` names
# the shim and says nothing about what ran, which is the question that went
# unasked while this gate reported 0. `rustup which` resolves it to the real
# executable, and that is what gets printed.
toolchain_report() {
  local tc b path
  for tc in stable 1.85.0; do
    printf '  toolchain %s\n' "$tc"
    for b in cargo rustc cargo-clippy; do
      path="$(rustup which --toolchain "$tc" "$b" 2>/dev/null)" \
        || path="(component not installed for this toolchain)"
      printf '    %-16s %s\n' "$b" "$path"
    done
    printf '    %-16s %s\n' "rustc --version" \
      "$(rustup run "$tc" rustc --version 2>&1 | tail -1)"
    # Asked separately rather than folded into the loop: a toolchain without the
    # clippy component makes `--version` print rustup's install *advice*, and a
    # version column containing a `help:` line is the kind of output this file
    # is otherwise careful not to produce. 1.85.0 legitimately has no clippy —
    # CI's msrv job does not install one either, because that job runs `check`.
    if rustup which --toolchain "$tc" cargo-clippy >/dev/null 2>&1; then
      printf '    %-16s %s\n' "clippy --version" \
        "$(rustup run "$tc" cargo-clippy --version 2>&1 | tail -1)"
    else
      printf '    %-16s %s\n' "clippy --version" "(n/a — no clippy on this toolchain)"
    fi
  done
  printf '  targets installed: %s\n' "$(rustup target list --installed | tr '\n' ' ')"
}

# Judged by exit code, as everything here is — but the OUTPUT is kept and
# printed WHEN THE STEP IS RED. Discarding it was defensible while a step was
# one command over one subject; with three clippy legs, "clippy exit=1" does not
# say which OS family or which lint, and a reader who must re-run the tool by
# hand to find out has been handed a verdict without its evidence.
step() {
  local name="$1"; shift
  local out code
  out="$("$@" 2>&1)"
  code=$?
  printf '  %-46s exit=%d\n' "$name" "$code"
  if [ "$code" -ne 0 ]; then
    printf '%s\n' "$out" | sed 's/^/      | /'
  fi
  return $code
}

# ONE `cargo test` RUN, and the verdict and the numbers come from IT.
#
# The first version ran it twice — once for the verdict, once to derive the
# counts — and the two disagreed: `run_gate` reported every step 0 while the
# second run reported 1 failed, and the script printed GREEN. A gate that
# contradicts itself and resolves the contradiction in favour of green is the
# failure this file was written against, arriving from a different direction.
TEST_OUT=""
TEST_CODE=0

# Which clippy legs went red, kept apart. `--sabotage` case 3 reads these,
# because "the gate went red" does not distinguish "the cross legs work" from
# "something else caught it and the cross legs are still unproven".
HOST_CLIPPY_CODE=0
CROSS_CLIPPY_CODE=0

run_gate() {
  local failed=0
  step "fmt --check" cargo fmt --all -- --check || failed=1

  # CI's line, verbatim: `--locked --workspace --all-targets --all-features --
  # -D warnings`, under `RUSTFLAGS=-D warnings`. This gate used to run
  # `--workspace --all-targets -- -D warnings` and nothing else: `--locked`,
  # `--all-features` and RUSTFLAGS were all absent. None of the three turned out
  # to be what hid plan.rs:454 — that was the target — but a gate whose flags
  # differ from CI's cannot be used to predict CI in either direction, so the
  # difference is closed rather than argued about.
  HOST_CLIPPY_CODE=0
  CROSS_CLIPPY_CODE=0
  local leg name triple
  for leg in $CLIPPY_LEGS; do
    name="${leg%%:*}"
    triple="${leg#*:}"
    # A target whose std is not installed makes cargo fail for a reason that has
    # nothing to do with this tree. Name it, and stay RED: a leg that quietly
    # does not run is precisely the green check this file exists to abolish.
    if [ -n "$triple" ] \
       && ! rustup target list --installed | grep -qx "$triple"; then
      printf '  %-46s exit=%d\n' "clippy $name ($triple)" 2
      printf '      | target std not installed — this leg DID NOT RUN.\n'
      printf '      | rustup target add %s\n' "$triple"
      failed=1
      CROSS_CLIPPY_CODE=2
      continue
    fi
    if RUSTFLAGS="-D warnings" step "clippy $name (${triple:-$(rustc -vV | awk '/^host:/ {print $2}')})" \
         cargo clippy --locked --workspace --all-targets --all-features \
         ${triple:+--target "$triple"} -- -D warnings
    then
      :
    else
      failed=1
      if [ "$name" = "host" ]; then HOST_CLIPPY_CODE=1; else CROSS_CLIPPY_CODE=1; fi
    fi
  done

  # **`cargo test` IS HOST-ONLY, AND CANNOT BE OTHERWISE HERE.** Running a
  # cross-compiled test needs an emulator or a machine of that OS, and there is
  # neither. The clippy legs above establish that `cfg(unix)` code COMPILES AND
  # LINTS CLEAN. They do not establish that it PASSES, and clean lints say
  # nothing about behaviour — so the local standing of every `cfg(unix)` control
  # is ZERO until a CI runner executes it.
  #
  # Registered with its instances in ADR-0002 §7, because a limit without them
  # is a disclaimer. Three whole controls are not taken here:
  #
  #   atomic_replace.rs:427   the_targets_unix_mode_survives_the_replacement —
  #                           the control for 662f8e5, the permissions repair,
  #                           which is also the commit that introduced the
  #                           defect this gate could not see. Its Windows twin
  #                           at :462 DOES run — and THE HALF THAT RUNS IS THE
  #                           HALF THAT CARRIES NO EXPOSURE. Windows models only
  #                           the read-only flag, which never governed who could
  #                           read the file; the absent half covers the Unix
  #                           mode, where 0600 coming back 0644 is the hazard
  #                           the primitive exists for. "Per-platform control,
  #                           green locally" is true and reassures about the
  #                           wrong member.
  #   ignore_state_git.rs:463 the reachable SIGKILL arm (ADR-0010 §10). The
  #                           mapping keeps synthesised-value coverage locally;
  #                           the reachable half does not.
  #   monitor_writer.rs:669   the cfg(not(windows)) arm of the reachability
  #                           premise under control (d), in
  #                           the_traversal_hazard_is_real_and_reachable_on_this_machine.
  #                           CI-only by construction. The cfg(windows) arm at
  #                           :652 is the one that runs here, and its reading
  #                           was already known.
  #
  # agents_store.rs:322 is NOT in that list. It is an inner block, not a whole
  # control: the test runs here and passes, because git-for-windows executes
  # hooks without an exec bit, and on Unix the `chmod` is protected by the
  # test's own positive control. Named anyway, because a reader grepping for
  # `cfg(unix)` will find it and deserves the answer rather than its absence.
  #
  # The label carries this, because the output is read without the source.
  TEST_OUT="$(mktemp -t gate.XXXXXX)"
  cargo test --locked --workspace --all-features >"$TEST_OUT" 2>&1
  TEST_CODE=$?
  printf '  %-46s exit=%d\n' "test --workspace (HOST ONLY)" "$TEST_CODE"
  [ "$TEST_CODE" -eq 0 ] || failed=1

  # **The label carries the scope, because the output is read without the
  # source.** This step is `check`, not `test`, and without `--all-targets`:
  # it compiles the LIB AND BINS on 1.85 and never looks at test code. That is
  # deliberate and CI says why -- `rust-version` promises what a consumer
  # builds, and dev-dependencies have MSRVs of their own.
  #
  # It is labelled here because of what `--sabotage` prints: against a tree
  # that does not compile, this step reported `exit=0`, correctly, about code
  # the sabotage never touched. A reader seeing "MSRV 0" beside "does not
  # compile" infers the wrong thing, and an instrument whose green is about a
  # different subject than the reader assumes is the class this repository
  # already catalogues. The verdict was still red -- clippy and test caught it
  # -- so this is a legibility repair, not a correctness one.
  #
  # The target is linux because CI's msrv job is ubuntu-latest. Same reasoning
  # as the clippy legs, and the same hole if it is left off.
  step "MSRV 1.85 lib+bins ($MSRV_TARGET)" \
    cargo +1.85.0 check --locked --workspace --target "$MSRV_TARGET" || failed=1
  return $failed
}

derive_numbers() {
  local out="$TEST_OUT"
  local code=$TEST_CODE
  [ -n "$out" ] || { echo "  (no test output — run_gate did not run)"; return 2; }
  # DERIVATIONS, stated because a number is only as good as where it came from:
  #   binaries  = lines matching '^test result:' — cargo prints one per test
  #               binary it RAN. It is 0 when nothing compiled, which is the
  #               whole point; the exit code above is what decides pass/fail.
  #   passed    = the sum of the "N passed" fields on those lines.
  #   failed    = the sum of the "N failed" fields.
  #   on / by   = the target these three describe, and the executable that
  #               produced them. Not decoration. The counts are about ONE
  #               triple, and this repository has code that only exists on the
  #               other two; omitting this is how "clippy 0" was read for 48
  #               commits as a statement about the repository.
  local binaries passed failed
  binaries=$(grep -c '^test result:' "$out")
  passed=$(awk '/^test result:/ { for (i=1;i<=NF;i++) if ($(i+1)=="passed;") s+=$i } END { print s+0 }' "$out")
  failed=$(awk '/^test result:/ { for (i=1;i<=NF;i++) if ($(i+1)=="failed;") s+=$i } END { print s+0 }' "$out")
  printf '  measured ON (target triple):                             %s\n' "$(rustc -vV | awk '/^host:/ {print $2}')"
  printf '  measured BY (resolved, not the PATH shim):               %s\n' "$(rustup which --toolchain stable cargo)"
  printf '  test binaries that RAN (count of "^test result:" lines): %s\n' "${binaries:-0}"
  printf '  tests passed (sum of "N passed" on those lines):         %s\n' "${passed:-0}"
  printf '  tests failed (sum of "N failed" on those lines):         %s\n' "${failed:-0}"
  printf '  cargo test exit code (the only thing that decides):      %s\n' "$code"
  return $code
}

# EVERY INJECTION BELOW IS RUSTFMT-CLEAN, and that is load-bearing rather than
# tidy. The earlier one-line `fn f() { .. }` form made `fmt --check` exit 1 in
# all three cases, so each sabotage went red partly for a reason it was not
# testing. For 1 and 2 that only blurred the reading. For 3 it would have
# INVALIDATED THE CONTROL: its whole claim is that the previous host-only gate
# could not see the defect, and a gate that catches it on formatting catches it.
# A control has to isolate the variable it is a control for.
sabotage() {
  local victim="crates/vibe-core/tests/atomic_replace.rs"
  local backup
  backup="$(mktemp -t sabotage.XXXXXX)"
  cp "$victim" "$backup"
  local overall=0

  echo "SABOTAGE 1 — a test that FAILS (compiles, asserts something false)"
  printf '\n#[test]\nfn deliberate_failure() {\n    assert_eq!(1, 2, "sabotage");\n}\n' >>"$victim"
  if run_gate; then
    echo "  *** THE GATE STAYED GREEN ON A FAILING TEST ***"
    overall=1
  else
    echo "  red, as required"
  fi
  cp "$backup" "$victim"

  echo
  echo "SABOTAGE 2 — a tree that DOES NOT COMPILE (the case that got through)"
  printf '\nfn deliberate_type_error() {\n    let _: u32 = "not a number";\n}\n' >>"$victim"
  if run_gate; then
    echo "  *** THE GATE STAYED GREEN ON A NON-COMPILING TREE ***"
    overall=1
  else
    echo "  red, as required"
  fi
  cp "$backup" "$victim"

  echo
  echo "SABOTAGE 3 — a defect that EXISTS ONLY ON UNIX (the round-12 case)"
  echo "  The gate before this one compiled the host target and nothing else, so"
  echo "  it was structurally incapable of seeing this. Red alone does NOT pass:"
  echo "  red for the wrong reason is also red. This control requires the HOST"
  echo "  leg GREEN and a CROSS leg RED, which is the only outcome that shows"
  echo "  the new capability is what caught it."
  printf '\n#[cfg(unix)]\nfn deliberate_unix_only_error() {\n    let _: u32 = "not a number";\n}\n' >>"$victim"
  run_gate
  local s3=$?
  if [ "$s3" -eq 0 ]; then
    echo "  *** THE GATE STAYED GREEN ON A UNIX-ONLY DEFECT ***"
    overall=1
  elif [ "$HOST_CLIPPY_CODE" -ne 0 ]; then
    echo "  *** RED, BUT THE HOST LEG WENT RED TOO. That does not demonstrate"
    echo "      cross-target coverage — the sabotage was not unix-only. ***"
    overall=1
  elif [ "$CROSS_CLIPPY_CODE" -eq 0 ]; then
    echo "  *** RED, BUT NOT FROM A CROSS CLIPPY LEG. Something else caught it"
    echo "      and the cross legs remain unproven. ***"
    overall=1
  else
    echo "  red from a cross leg with the host leg green, as required"
  fi
  cp "$backup" "$victim"
  rm -f "$backup"

  echo
  if [ "$overall" -eq 0 ]; then
    echo "all three sabotages produced red, each for its own reason;"
    echo "the gate's channel is measured"
  else
    echo "THE GATE IS NOT TRUSTWORTHY — see above"
  fi
  return $overall
}

echo "gate on $(git rev-parse --abbrev-ref HEAD) @ $(git rev-parse --short HEAD)"
echo
echo "toolchains invoked below, resolved through rustup rather than read off PATH:"
toolchain_report
echo

if [ "${1:-}" = "--sabotage" ]; then
  sabotage
  exit $?
fi

run_gate
gate_code=$?
echo
derive_numbers
rm -f "$TEST_OUT"
echo
if [ "$gate_code" -eq 0 ]; then
  echo "GREEN — every step exited 0"
else
  echo "RED — at least one step did not exit 0"
fi
exit $gate_code
