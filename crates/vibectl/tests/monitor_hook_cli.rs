//! Controls for `vibe monitor hook` — the process Claude Code actually spawns.
//!
//! These run the **real binary** with a payload on stdin, because everything
//! this path is shaped by lives outside the library: the exit status, stdin, the
//! panic reporter, and `clap`'s own exit code. A unit test over `hook_main`
//! would assert against none of them.
//!
//! # The external exit contract
//!
//! `vibectl`'s contract assigns 2 = partial. Claude Code reads a hook's exit 2
//! as a **blocking** error fed back to the agent, which ADR-0011 §7a forbids:
//! the monitor is additive and *an observer that can stop the subject is not
//! one*. So every control here asserts the code is 0 or 1 and **never 2**,
//! including on the paths where `clap` would have produced it.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Run the hook with `payload` on stdin. Returns (code, stdout, stderr).
fn run_hook(args: &[&str], payload: &str) -> (Option<i32>, String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_vibe"))
        .args(["monitor", "hook"])
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn vibe");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(payload.as_bytes())
        .expect("write payload");
    let out = child.wait_with_output().expect("wait");
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn payload(session: &str, event: &str) -> String {
    format!(r#"{{"session_id":"{session}","hook_event_name":"{event}","cwd":"/tmp/p"}}"#)
}

fn only_file(dir: &Path) -> std::path::PathBuf {
    let mut v: Vec<_> = std::fs::read_dir(dir)
        .expect("sink readable")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .collect();
    v.sort();
    assert_eq!(v.len(), 1, "expected exactly one record file, got {v:?}");
    v.remove(0)
}

// ---------------------------------------------------------------------------
// The delivering path
// ---------------------------------------------------------------------------

/// **A payload on stdin becomes a record on disk, and the exit says so.**
#[test]
fn a_payload_on_stdin_is_written_and_the_hook_exits_zero() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sink = dir.path().join("sink");

    let (code, _out, err) = run_hook(
        &[
            "--identity",
            "alpha",
            "--sink",
            sink.to_str().expect("utf8"),
            "--contract",
            "1",
        ],
        &payload("sess-1", "SessionStart"),
    );

    assert_eq!(code, Some(0), "stderr was: {err}");

    let path = only_file(&sink);
    assert_eq!(
        path.file_name().and_then(|n| n.to_str()),
        Some("sess-1__alpha.jsonl"),
        "session-level records are two components"
    );

    let text = std::fs::read_to_string(&path).expect("read");
    assert!(text.ends_with('\n'), "the frame is the trailing newline");
    let v: serde_json::Value = serde_json::from_str(text.trim_end()).expect("one JSON line");
    assert_eq!(v.get("identity").and_then(|x| x.as_str()), Some("alpha"));
    assert_eq!(v.get("session").and_then(|x| x.as_str()), Some("sess-1"));
    assert_eq!(
        v.get("event").and_then(|x| x.as_str()),
        Some("SessionStart")
    );
    assert_eq!(
        v.get("sink").and_then(|x| x.as_str()),
        sink.to_str(),
        "the record carries the sink AS RECEIVED, so an install-versus-write \
         disagreement about WHERE is visible rather than inferred from a file \
         nobody can find"
    );
    assert!(
        v.get("payload").and_then(|x| x.as_str()).is_some(),
        "the payload is stored verbatim as a string"
    );
}

/// A subagent payload lands in a three-component file, through the real binary.
#[test]
fn a_subagent_payload_lands_in_an_agent_scoped_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sink = dir.path().join("sink");
    let p =
        r#"{"session_id":"sess-1","agent_id":"ab8b50189992e6091","hook_event_name":"PreToolUse"}"#;

    let (code, _o, err) = run_hook(
        &[
            "--identity",
            "alpha",
            "--sink",
            sink.to_str().expect("utf8"),
            "--contract",
            "1",
        ],
        p,
    );
    assert_eq!(code, Some(0), "stderr: {err}");
    assert_eq!(
        only_file(&sink).file_name().and_then(|n| n.to_str()),
        Some("sess-1__ab8b50189992e6091__alpha.jsonl")
    );
}

// ---------------------------------------------------------------------------
// The failing paths — exit 1, a diagnostic, and never 2
// ---------------------------------------------------------------------------

/// **Paired against the delivering test: a write that cannot land exits 1 with
/// a non-empty diagnostic, and writes nothing.**
///
/// The sink path is blocked by a *file*, so `create_dir_all` fails — the same
/// reachable failure the library control uses, driven here through the real
/// process so the exit code is part of the assertion.
#[test]
fn a_failed_write_exits_one_with_a_diagnostic_and_leaves_nothing_behind() {
    let dir = tempfile::tempdir().expect("tempdir");
    let blocked = dir.path().join("sink");
    std::fs::write(&blocked, b"I am a file, not a directory").expect("plant");

    let (code, _o, err) = run_hook(
        &[
            "--identity",
            "alpha",
            "--sink",
            blocked.to_str().expect("utf8"),
            "--contract",
            "1",
        ],
        &payload("sess-1", "SessionStart"),
    );

    assert_eq!(code, Some(1), "a failed write exits 1");
    assert_ne!(
        code,
        Some(2),
        "and never 2, which the agent reads as blocking"
    );
    assert!(
        !err.trim().is_empty(),
        "a failed write must say so; silence here is non-delivery arriving from \
         our own side, which is §7's hazard produced by the mechanism installed \
         to prevent it"
    );
    assert!(
        err.contains("LOST"),
        "the diagnostic must say the event is gone, not merely that a call \
         failed: {err}"
    );
}

/// Empty stdin is its own refusal, distinct from malformed bytes.
///
/// Measured: Claude Code 2.1.233 warns *"no stdin data received in 3s"*, so
/// this arm is reachable rather than defensive. *The channel gave us nothing*
/// and *the agent's payload is broken* have different remedies.
#[test]
fn empty_stdin_is_reported_as_no_payload_and_not_as_malformed_json() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sink = dir.path().join("sink");
    let args = [
        "--identity",
        "alpha",
        "--sink",
        sink.to_str().expect("utf8"),
        "--contract",
        "1",
    ];

    let (code, _o, err) = run_hook(&args, "");
    assert_eq!(code, Some(1));
    assert!(err.contains("no_payload"), "got: {err}");

    // Paired: malformed bytes are a DIFFERENT reason, or the two are merged.
    let (code2, _o2, err2) = run_hook(&args, "this is not json");
    assert_eq!(code2, Some(1));
    assert!(err2.contains("not_json"), "got: {err2}");
    assert!(
        !err2.contains("no_payload"),
        "an unparseable payload is not an absent one: {err2}"
    );
}

/// An identity that is not a path component is refused **at write**, because
/// §7 permits hand-installed hooks that install never sees.
#[test]
fn a_hostile_identity_is_refused_at_write_and_writes_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sink = dir.path().join("sink");

    let (code, _o, err) = run_hook(
        &[
            "--identity",
            "x/../../escape",
            "--sink",
            sink.to_str().expect("utf8"),
            "--contract",
            "1",
        ],
        &payload("sess-1", "SessionStart"),
    );
    assert_eq!(code, Some(1));
    assert!(err.contains("path component"), "got: {err}");
    assert!(
        !sink.exists(),
        "a refused identity must not even create the sink"
    );
    // Nothing escaped either.
    let outside: Vec<_> = std::fs::read_dir(dir.path())
        .expect("readable")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .collect();
    assert!(outside.is_empty(), "something was written: {outside:?}");
}

/// **A usage error exits 1, not `clap`'s 2.**
///
/// This is the control for the whole raw-argv dispatch. Without it the hook
/// path would hand Claude Code a blocking error for a typo in a settings file.
#[test]
fn a_usage_error_exits_one_rather_than_claps_two() {
    let (code, _o, err) = run_hook(&["--identity", "alpha", "--no-such-flag"], "");
    assert_eq!(
        code,
        Some(1),
        "clap's own exit code for a usage error is 2, and 2 is the one code \
         this path may not emit. stderr: {err}"
    );
    assert!(
        !err.trim().is_empty(),
        "the usage error must still be explained"
    );

    // Paired: a well-formed invocation is NOT reported as a usage error, or
    // this is satisfied by a build that rejects everything.
    let dir = tempfile::tempdir().expect("tempdir");
    let sink = dir.path().join("sink");
    let (ok, _o2, _e2) = run_hook(
        &[
            "--identity",
            "alpha",
            "--sink",
            sink.to_str().expect("utf8"),
            "--contract",
            "1",
        ],
        &payload("sess-1", "SessionStart"),
    );
    assert_eq!(ok, Some(0));
}

/// Missing required arguments is also a usage error, and also not 2.
#[test]
fn missing_required_arguments_exit_one() {
    let (code, _o, _e) = run_hook(&[], "");
    assert_eq!(code, Some(1));
}

// ---------------------------------------------------------------------------
// The panic reporter
// ---------------------------------------------------------------------------

/// **The panic reporter fires, observed rather than assumed installed.**
///
/// A panic exits 101 with no record: the event is lost, the agent continues,
/// and nothing else reports it. That is §7's non-delivery hazard produced by
/// our own crash, so the loss has to be *stated*.
///
/// `--panic-probe` is an **argument**, not an environment variable, so the
/// trigger cannot arrive ambiently — the objection ADR-0008 §9 raised against a
/// build flag, which `RUSTFLAGS` and `.cargo/config.toml` inherit from parent
/// directories. An argument has to be typed by someone.
#[test]
fn a_panic_is_reported_on_stderr_rather_than_dying_silently() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sink = dir.path().join("sink");
    let (code, _o, err) = run_hook(
        &[
            "--identity",
            "alpha",
            "--sink",
            sink.to_str().expect("utf8"),
            "--contract",
            "1",
            "--panic-probe",
        ],
        &payload("sess-1", "SessionStart"),
    );

    assert_ne!(code, Some(0), "a panic is not a delivery");
    assert_ne!(code, Some(2), "and still never 2");
    assert!(
        err.contains("PANIC"),
        "the panic reporter did not fire — a hook that dies silently is the \
         exact failure this reporter exists against, and a reporter that is \
         merely INSTALLED looks identical to one that works. stderr: {err}"
    );
    assert!(
        err.contains("NOT written"),
        "the report must say the event was lost: {err}"
    );

    // Paired: an ordinary run emits no panic line, or the assertion above is
    // satisfied by a build that prints PANIC unconditionally.
    let dir2 = tempfile::tempdir().expect("tempdir");
    let sink2 = dir2.path().join("sink");
    let (ok, _o2, clean) = run_hook(
        &[
            "--identity",
            "alpha",
            "--sink",
            sink2.to_str().expect("utf8"),
            "--contract",
            "1",
        ],
        &payload("sess-1", "SessionStart"),
    );
    assert_eq!(ok, Some(0));
    assert!(!clean.contains("PANIC"), "stderr: {clean}");
}

// ---------------------------------------------------------------------------
// The contract version — the write half
// ---------------------------------------------------------------------------

/// A declared contract that differs from the one this binary implements is
/// **reported and never repaired**, and the record is still written.
///
/// §7: a mismatch is reported, never repaired — repair would be sync under
/// another name, done at the moment the user is least able to see it. Losing
/// the event would be worse than writing it, so the record lands stamped with
/// what actually produced the bytes.
#[test]
fn a_contract_mismatch_is_reported_and_the_record_is_still_written() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sink = dir.path().join("sink");

    let (code, _o, err) = run_hook(
        &[
            "--identity",
            "alpha",
            "--sink",
            sink.to_str().expect("utf8"),
            "--contract",
            "99",
        ],
        &payload("sess-1", "SessionStart"),
    );
    assert_eq!(code, Some(0), "a mismatch does not lose the event");
    assert!(err.contains("contract mismatch"), "stderr: {err}");
    assert!(
        err.contains("Nothing is repaired"),
        "the report must say what it did NOT do: {err}"
    );

    let text = std::fs::read_to_string(only_file(&sink)).expect("read");
    let v: serde_json::Value = serde_json::from_str(text.trim_end()).expect("json");
    assert_eq!(
        v.get("v").and_then(|x| x.as_str()),
        Some("1"),
        "the record is stamped by the binary that wrote it, not by what the \
         config claimed"
    );

    // Paired: a matching contract reports nothing.
    let dir2 = tempfile::tempdir().expect("tempdir");
    let sink2 = dir2.path().join("sink");
    let (_c, _o, clean) = run_hook(
        &[
            "--identity",
            "alpha",
            "--sink",
            sink2.to_str().expect("utf8"),
            "--contract",
            "1",
        ],
        &payload("sess-1", "SessionStart"),
    );
    assert!(!clean.contains("contract mismatch"), "stderr: {clean}");
}

// ---------------------------------------------------------------------------
// Two invocations, one file
// ---------------------------------------------------------------------------

/// Two runs of the real binary append to the same file rather than truncating
/// it — the property the whole transport rests on.
#[test]
fn a_second_invocation_appends_rather_than_truncating() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sink = dir.path().join("sink");
    let args = [
        "--identity",
        "alpha",
        "--sink",
        sink.to_str().expect("utf8"),
        "--contract",
        "1",
    ];

    assert_eq!(run_hook(&args, &payload("s", "SessionStart")).0, Some(0));
    assert_eq!(run_hook(&args, &payload("s", "SessionEnd")).0, Some(0));

    let text = std::fs::read_to_string(only_file(&sink)).expect("read");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "the second run appended: {text}");
    assert!(lines[0].contains("SessionStart"));
    assert!(lines[1].contains("SessionEnd"));
}

// ---------------------------------------------------------------------------
// Cold-start duration — the input to the `timeout` value, on every platform
// ---------------------------------------------------------------------------

/// **How long one cold hook invocation takes, measured where CI can see it.**
///
/// ADR-0011 §7b writes an explicit `timeout` rather than depending on a default
/// whose bound §2 could not locate. The value has to come from a measurement of
/// this binary, and it has to come from **every platform**, because §9's
/// declared limit is that everything else in that section was taken on
/// `win32-x64`. This one needs no live agent — the hook is a process that reads
/// stdin and appends — so unlike the rest of the round-3 corpus it mechanizes,
/// and the number appears in the ordinary test job on all three runners.
///
/// # What is measured, and what that is not
///
/// The whole invocation as Claude Code experiences it: process spawn, `clap`
/// parse, stdin read, the append, exit. **It is not the write.** The write is
/// one syscall (see the writer's docs); almost all of this is process startup,
/// which is why it is the quantity a `timeout` has to clear.
///
/// # Why this asserts a ceiling rather than only printing
///
/// A test that measures and asserts nothing is a report nobody reads, and a
/// report nobody reads is how a regression ships. But a tight timing assertion
/// on a shared CI runner fails for reasons that have nothing to do with this
/// code, which is the flake ADR-0002 §7 rejects — a control that goes red
/// without a defect trains people to ignore it.
///
/// So the ceiling is deliberately **enormous** relative to the expected value:
/// it is not a performance budget, it is a tripwire for the case where the hook
/// has started doing something it must not, like waiting on a network or a
/// lock. The real output is the printed maximum, which CI carries per platform.
#[test]
fn a_cold_hook_invocation_is_measured_and_reported_per_platform() {
    /// Not a budget. A hook that takes longer than this is not slow, it is
    /// doing something it was never meant to do.
    const TRIPWIRE: Duration = Duration::from_secs(10);
    const RUNS: usize = 10;

    let dir = tempfile::tempdir().expect("tempdir");
    let sink = dir.path().join("sink");

    let mut durations = Vec::with_capacity(RUNS);
    for i in 0..RUNS {
        let started = Instant::now();
        let (code, _out, err) = run_hook(
            &[
                "--identity",
                "coldstart",
                "--sink",
                sink.to_str().expect("utf8"),
                "--contract",
                "1",
            ],
            &payload(&format!("cold-{i}"), "SessionStart"),
        );
        let elapsed = started.elapsed();
        // A run that failed measures a failure path, not a cold start.
        assert_eq!(code, Some(0), "run {i} did not deliver: {err}");
        durations.push(elapsed);
    }

    durations.sort_unstable();
    let min = durations[0];
    let median = durations[RUNS / 2];
    let max = durations[RUNS - 1];

    // Printed unconditionally, and visible in CI with `--nocapture`; the
    // assertion below is not where the information is.
    println!(
        "cold-start `vibe monitor hook` on {os}/{arch}: \
         min {min:?}, median {median:?}, max {max:?} over {RUNS} runs",
        os = std::env::consts::OS,
        arch = std::env::consts::ARCH,
    );

    assert!(
        max < TRIPWIRE,
        "a cold hook invocation took {max:?} on {}/{}, past the {TRIPWIRE:?} \
         tripwire. This is not a performance budget — at this magnitude the \
         hook is waiting on something, and ADR-0011 §7b's `timeout` is chosen \
         against this measurement.",
        std::env::consts::OS,
        std::env::consts::ARCH,
    );

    // Paired: the measurement is only meaningful if the runs actually did the
    // work. Ten deliveries into one sink for ten sessions is ten files.
    let files = std::fs::read_dir(&sink).expect("sink readable").count();
    assert_eq!(
        files, RUNS,
        "the timed runs must have written what they were timed for, or this \
         measures a hook that exited early"
    );
}
