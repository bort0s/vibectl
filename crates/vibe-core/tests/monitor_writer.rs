//! Negative controls for the monitor writer — ADR-0011 §9 (a) through (f).
//!
//! Every guard here was sabotaged and observed to go **red** before the code
//! was committed. ADR-0002 §7's whole history is that intending to get this
//! right has not been enough: three preservation tests, one unreached guard,
//! one racing control and one branch-coverage gap all looked exactly like
//! proof and proved nothing.
//!
//! # FORBIDDEN, and it is written here because here is where it would be typed
//!
//! **Do not write a control asserting that an open `tool_use_id` renders as
//! working.** ADR-0011 §5's round-2 measurement retracted that claim entirely:
//! a *denied* tool leaves its `tool_use_id` open **permanently**, in a session
//! that has already emitted `Stop` and `SessionEnd`. An open id also covers a
//! 9-second approval wait that is byte-identical to a 9-second slow tool. So
//! the disjunction is *executing*, *waiting for approval*, or *finished after a
//! denial* — three states with no common consequence, which is an absence of
//! information with a list inside it rather than a narrowing.
//!
//! Such a control **will pass, and passing is the defect**: it would certify a
//! build that is wrong on 2.1.233. ADR-0002 §7 carries the general rule — *a
//! retraction removes a control's subject, and nothing inherits the removal;
//! record what must not be built beside what must* — and this paragraph is that
//! record, placed where the next person writing a control is standing.
//!
//! # This file is deliberately not gated on a `VIBE_REQUIRE_*` variable
//!
//! It needs no external tool. Adding a gate would push
//! `control_inventory.rs`'s derived count toward ADR-0008 §9's revisit trigger
//! for no gain, and that trigger must fire on a real seventh control rather
//! than on bookkeeping.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use vibe_core::monitor::{
    ComponentRejection, FixedStamps, NotPrunableReason, PayloadRefusal, Prunability, ReadRecord,
    SessionComponent, SinkRead, StampSource, SystemStamps, TailState, WriteOutcome, WriteStage,
    Writer, WriterIdentity, collisions, file_key, read_file,
};

// ---------------------------------------------------------------------------
// Fixture helpers
// ---------------------------------------------------------------------------

fn stamps() -> Arc<dyn StampSource> {
    // A long, ascending run so no test can exhaust it and silently take the
    // unstamped arm while claiming to test the stamped one.
    Arc::new(FixedStamps::new((1..500).map(|n| n * 1_000_000).collect()))
}

fn writer(sink: &Path, identity: &str) -> Writer {
    Writer::new(
        sink,
        WriterIdentity::parse(identity).expect("fixture identity is valid by construction"),
        stamps(),
    )
}

fn payload(session: &str, event: &str) -> String {
    format!(r#"{{"session_id":"{session}","hook_event_name":"{event}","cwd":"/tmp/p"}}"#)
}

fn written_path(outcome: &WriteOutcome) -> PathBuf {
    match outcome {
        WriteOutcome::Written { path, .. } => path.clone(),
        other => panic!("expected a written record, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// §9 (a) — Distinct paths, paired, on THREE axes rather than two
// ---------------------------------------------------------------------------

/// ADR-0011 §9 (a). The two-axis version of this control **had the defect it
/// was written to prevent**: it used one hook per settings source, so it would
/// have passed against a build keying on the settings file — which is the
/// defect that actually shipped.
///
/// The axes are:
///   1. two **sessions**, one hook each;
///   2. two **settings sources** within one session;
///   3. **two hooks declared in the same settings file** for the same event.
///
/// Axis 3 is the one that separates *"one file per declared identity"* from
/// *"one file per settings file"*, and no fixture with one hook per source can
/// reach it. ADR-0002 §7: *a fixture with N=1 cannot tell "one" from "exactly
/// one"* — every `per` in the design is a quantifier needing its own fixture.
#[test]
fn every_combination_of_the_three_axes_produces_a_distinct_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sink = dir.path();

    // Modelled as the writer actually receives them: a hook declares its own
    // identity, and the settings file it came from is NOT an input. The source
    // column exists so the sabotage below can key on it.
    let hooks = [
        ("session-a", "settings", "alpha"), // axis 1 against row 3
        ("session-b", "settings", "alpha"), // axis 1
        ("session-a", "local", "beta"),     // axis 2: second source, same session
        ("session-a", "settings", "gamma"), // axis 3: SECOND HOOK, SAME FILE
    ];

    let mut paths = Vec::new();
    for (session, _source, identity) in hooks {
        let outcome = writer(sink, identity).append(&payload(session, "SessionStart"));
        paths.push(written_path(&outcome));
    }

    // Positive control: the writes actually happened. Without this, a build
    // producing four identical *nonexistent* paths would satisfy nothing while
    // the uniqueness assertion below still needed real files to be wrong about.
    for p in &paths {
        assert!(p.is_file(), "no file at {}", p.display());
    }

    let mut unique: Vec<&PathBuf> = paths.iter().collect();
    unique.sort();
    unique.dedup();
    assert_eq!(
        unique.len(),
        paths.len(),
        "two hooks shared a file. Concurrent append has returned inside the \
         design whose entire justification is that it cannot occur.\n{paths:#?}"
    );

    // Axis 3 stated as its own assertion, so a future edit that collapses it
    // fails here by name rather than in an aggregate count.
    assert_ne!(
        paths[0], paths[3],
        "two hooks declared in the SAME settings file for the SAME session \
         shared a file — the axis a one-hook-per-source fixture cannot see"
    );
}

/// The second sabotage ADR-0011 §9 (a) asks for, expressed as a fixture rather
/// than as an edit to the crate — **because the defect it models is
/// unrepresentable in this design, and that is the finding.**
///
/// A build keying on the settings source cannot be produced by sabotaging
/// [`Writer`]: the settings source is never passed to it. So the defect is
/// modelled where it would actually live, in the choice of what becomes the
/// identity, and this test asserts the diagnostic property the ADR predicts —
/// **only the third axis goes red.** That is what establishes axis 3 is doing
/// work the other two cannot.
#[test]
fn keying_on_the_settings_source_leaves_axes_one_and_two_green_and_only_axis_three_red() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sink = dir.path();

    // The sabotage: identity := settings source.
    let keyed = |session: &str, source: &str| {
        written_path(&writer(sink, source).append(&payload(session, "SessionStart")))
    };

    let axis1_a = keyed("session-a", "settings");
    let axis1_b = keyed("session-b", "settings");
    let axis2_a = keyed("session-a", "settings");
    let axis2_b = keyed("session-a", "local");
    let axis3_a = keyed("session-a", "settings");
    let axis3_b = keyed("session-a", "settings"); // second hook, same file

    assert_ne!(axis1_a, axis1_b, "axis 1 must survive this sabotage");
    assert_ne!(axis2_a, axis2_b, "axis 2 must survive this sabotage");
    assert_eq!(
        axis3_a, axis3_b,
        "axis 3 must COLLIDE under this sabotage — if it does not, this test is \
         no longer modelling the shipped defect and axis 3's control proves \
         nothing about it"
    );
}

// ---------------------------------------------------------------------------
// §9 (b) — A cut-mid-record tail
// ---------------------------------------------------------------------------

/// ADR-0011 §9 (b). The reader must **report** a partial tail — not drop it
/// silently, and not parse a prefix that happens to be valid.
///
/// The fixture is constructible by hand precisely because one-writer-per-file
/// bounds the hazard positionally: only the last record can be partial. It
/// needs no race, which is the property that made the rejected shared-file
/// shape inadmissible — a failure that cannot be induced on demand cannot have
/// a paired control.
#[test]
fn a_truncated_final_record_is_reported_as_a_partial_tail() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("s__ident.jsonl");

    fs::write(
        &path,
        "{\"v\":\"1\",\"session\":\"s\",\"event\":\"SessionStart\"}\n\
         {\"v\":\"1\",\"session\":\"s\",\"even",
    )
    .expect("write fixture");

    let SinkRead::Read(file) = read_file(&path) else {
        panic!("fixture must be readable");
    };

    assert_eq!(
        file.records.len(),
        1,
        "the one whole record must still be read; a torn tail is not a reason \
         to discard the records before it"
    );
    assert!(
        matches!(file.tail, TailState::Partial { .. }),
        "a file not ending on a record boundary has a torn tail, got {:?}",
        file.tail
    );
}

/// The half that makes (b) about framing rather than about parsing.
///
/// The truncated bytes here are **valid JSON on their own**. A reader testing
/// *"does the last line parse"* accepts them as a complete record and the loss
/// is silent. The newline frame is what makes the answer certain instead of
/// heuristic.
#[test]
fn a_truncated_record_that_happens_to_be_valid_json_is_still_a_partial_tail() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("s__ident.jsonl");

    let torn = r#"{"v":"1","session":"s","event":"SessionStart"}"#;
    serde_json::from_str::<serde_json::Value>(torn)
        .expect("the fixture's premise: these bytes ARE valid JSON, or this test is not the one it claims to be");

    fs::write(&path, format!("{torn}\n{torn}")).expect("write fixture");

    let SinkRead::Read(file) = read_file(&path) else {
        panic!("fixture must be readable");
    };
    assert_eq!(file.records.len(), 1);
    assert_eq!(
        file.tail,
        TailState::Partial { bytes: torn.len() },
        "valid-looking bytes with no terminating newline are a torn tail"
    );
}

/// Paired: a whole file must read as complete, or the assertion above is
/// satisfied by a reader that calls everything partial.
#[test]
fn a_whole_file_reads_as_complete() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sink = dir.path();
    let w = writer(sink, "ident");
    let path = written_path(&w.append(&payload("sess", "SessionStart")));
    w.append(&payload("sess", "SessionEnd"));

    let SinkRead::Read(file) = read_file(&path) else {
        panic!("readable");
    };
    assert_eq!(file.tail, TailState::Complete);
    assert_eq!(file.records.len(), 2);
}

// ---------------------------------------------------------------------------
// §9 (c) — Uniqueness is refused, paired
// ---------------------------------------------------------------------------

/// ADR-0011 §9 (c). A config declaring the same identity twice must be refused;
/// the same config with distinct identities must be accepted and produce two
/// files.
///
/// **The fixture reaches the check rather than failing earlier.** Both declared
/// identities are charset-valid, so a config that is malformed for some other
/// reason cannot be what produces the refusal — the unreached-guard rule
/// (ADR-0002 §7), which cost this project a `git fetch` that errored several
/// steps before the guard it was meant to prove.
#[test]
fn a_duplicated_identity_is_refused_and_distinct_ones_are_accepted() {
    let duplicated = vec!["alpha".to_owned(), "beta".to_owned(), "alpha".to_owned()];
    for d in &duplicated {
        assert!(
            WriterIdentity::parse(d).is_ok(),
            "the fixture must reach the uniqueness check: {d} is not even a \
             valid identity, so a refusal would say nothing about uniqueness"
        );
    }

    let found = collisions(&duplicated);
    assert_eq!(found.len(), 1, "expected one collision, got {found:#?}");
    assert_eq!(found[0].file_key, "alpha");
    assert_eq!(found[0].declared.len(), 2);

    // Paired.
    let distinct = vec!["alpha".to_owned(), "beta".to_owned()];
    assert!(
        collisions(&distinct).is_empty(),
        "distinct identities must be accepted, or the check above is satisfied \
         by a build that refuses everything"
    );

    // …and they produce two files.
    let dir = tempfile::tempdir().expect("tempdir");
    let a = written_path(&writer(dir.path(), "alpha").append(&payload("s", "SessionStart")));
    let b = written_path(&writer(dir.path(), "beta").append(&payload("s", "SessionStart")));
    assert_ne!(a, b);
}

// ---------------------------------------------------------------------------
// §9 (d) — The identity is validated as a path component, paired
// ---------------------------------------------------------------------------

/// ADR-0011 §9 (d). A traversal, a separator and a `:` must each be refused;
/// a valid identity must be accepted and produce a file.
///
/// Validated **at install and at write**, not at install alone: §7 permits
/// hand-installed hooks, which install never sees. Here both are the same
/// function reached through the same type, which is the point — see
/// [`identity`](vibe_core::monitor::identity) for why a `WriterIdentity`
/// cannot be constructed from an unchecked string.
#[test]
fn a_traversal_a_separator_and_a_colon_are_refused_as_identities() {
    for hostile in [
        "../escape",
        "..\\escape",
        "a/b",
        "a\\b",
        "a:b",
        "a*b",
        "a?b",
        "a|b",
        "a<b",
        "a>b",
        "a\"b",
        "ok.",
        "ok ",
        "",
    ] {
        assert!(
            WriterIdentity::parse(hostile).is_err(),
            "{hostile:?} was accepted as an identity"
        );
    }

    // Paired: a valid identity is accepted AND produces a file. Without this
    // half, a build rejecting every identity passes the loop above perfectly.
    let dir = tempfile::tempdir().expect("tempdir");
    let outcome = writer(dir.path(), "ok-1_A").append(&payload("s", "SessionStart"));
    let path = written_path(&outcome);
    assert!(path.is_file());
    assert_eq!(
        path.parent(),
        Some(dir.path()),
        "an accepted identity must land inside the sink"
    );
}

/// The session component is the other half of the same filename and ADR-0011
/// §7a is silent about it. Recorded as a gap in the spec rather than as an
/// extension of it — and validated, because data reaching a path is data
/// reaching a path whichever side of the `__` it sits on.
#[test]
fn a_hostile_session_id_is_refused_rather_than_reaching_a_filename() {
    let dir = tempfile::tempdir().expect("tempdir");
    let w = writer(dir.path(), "ident");

    for hostile in ["../escape", "a/b", "a:b", ""] {
        let outcome = w.append(&payload(hostile, "SessionStart"));
        assert!(
            matches!(
                outcome,
                WriteOutcome::Refused {
                    reason: PayloadRefusal::SessionRejected { .. }
                }
            ),
            "session {hostile:?} was not refused: {outcome:?}"
        );
    }

    // Paired: an ordinary session id is accepted.
    assert!(
        w.append(&payload("0f9c4d2e-uuid-like", "SessionStart"))
            .delivered()
    );
}

/// **The hazard the charset check guards is real and reachable — measured, not
/// reasoned about.**
///
/// ADR-0011 §9 (d) says the sabotage must show the traversal case *writes
/// outside the sink*, *"which is the assertion that makes this about
/// containment rather than about a rejected string."* That is only a
/// containment claim if the escape is actually producible; against an
/// unreachable hazard, removing the check would change nothing and the control
/// would be green for the wrong reason.
///
/// So this asks the filesystem, at the layer the product uses — `Path::join`
/// plus `fs::write` — rather than reasoning about it or measuring it in another
/// language. ADR-0002 §7: *an instrument's properties are measured with the
/// instrument*, and it belongs to a build rather than to a category.
///
/// # Measured on Windows 10 Pro 19045, 2026-08-18
///
/// | identity | result |
/// | --- | --- |
/// | `../escape`, `..\escape`, `/escape` | `NotFound` (os 3) — **no escape** |
/// | `x/../../escape`, `x\..\..\escape` | **OK, and a file appears in the sink's parent** |
/// | `a:b` | OK, and **no file exists** — an NTFS alternate data stream |
///
/// **The naive traversal does not escape, and that is the trap.** `sess__..`
/// is a literal directory name, not a parent reference, because the identity is
/// always flanked by `<session>__` and `.jsonl`. A fixture testing only
/// `../escape` observes the write fail and reads as proof of containment. The
/// escaping form puts the `..` in a *middle* segment, where nothing flanks it —
/// and Windows canonicalises it lexically, so the nonexistent `sess__x`
/// directory is no obstacle at all.
///
/// Whether the middle-segment form also escapes on Linux and macOS is
/// **unmeasured here**: those resolve `..` against real directories, so a
/// nonexistent `sess__x` may well produce `ENOENT` instead. The platform
/// asymmetry is recorded as unknown rather than assumed either way.
#[test]
fn the_traversal_hazard_is_real_and_reachable_on_this_machine() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sink = dir.path().join("sink");
    fs::create_dir_all(&sink).expect("sink");
    let before = list(dir.path());

    for hostile in [
        "../escape",
        "..\\escape",
        "x/../../escape",
        "x\\..\\..\\escape",
        "a:b",
        "/escape",
    ] {
        // Exactly what `Writer::record_path` composes, with the check removed.
        let path = sink.join(format!("sess__{hostile}.jsonl"));
        let result = fs::write(&path, b"{}\n");
        println!(
            "  identity {hostile:22?}\n      write: {}",
            match &result {
                Ok(()) => "OK".to_owned(),
                Err(e) => format!("{:?} os={:?}", e.kind(), e.raw_os_error()),
            }
        );
    }

    let escaped: Vec<PathBuf> = list(dir.path())
        .into_iter()
        .filter(|p| !before.contains(p))
        .collect();
    println!("  appeared OUTSIDE the sink: {escaped:#?}");

    // The premise of control (d). If this ever stops holding, the charset check
    // is guarding an unreachable hazard on this platform and the *containment*
    // control below becomes untestable here — which is a finding, not a pass.
    assert!(
        !escaped.is_empty(),
        "no unvalidated identity escaped the sink, so on this platform the \
         traversal hazard is not reachable through this filename scheme and \
         the containment control cannot be demonstrated here. That is a result \
         to record, not a green to accept."
    );
}

/// **§9 (d)'s containment control.** Nothing hostile may leave the sink,
/// whichever layer stops it.
///
/// Written so the assertion is on **where the bytes land**, not on which
/// function returned an error. In the shipped build the identity is refused and
/// no write is attempted; with the charset check sabotaged the identity parses,
/// the writer composes a path, and the escaping form measured above puts a file
/// in the sink's parent — turning this red. That is what makes it a containment
/// control rather than a test that a string was rejected.
#[test]
fn no_hostile_identity_escapes_the_sink_through_the_writer() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sink = dir.path().join("sink");
    fs::create_dir_all(&sink).expect("sink");
    let before = list(dir.path());

    let hostile = [
        "../escape",
        "..\\escape",
        "x/../../escape",
        "x\\..\\..\\escape",
        "a:b",
        "/escape",
        "ok.",
        "ok ",
        "",
    ];

    for h in hostile {
        match WriterIdentity::parse(h) {
            Err(_) => {} // refused before a path exists — the shipped path
            Ok(identity) => {
                // Only reachable with the charset check removed. Let it write,
                // so the assertion below observes the real consequence.
                let w = Writer::new(&sink, identity, stamps());
                let outcome = w.append(&payload("sess", "SessionStart"));
                println!("  {h:?} PARSED, wrote: {outcome:?}");
            }
        }
    }

    let escaped: Vec<PathBuf> = list(dir.path())
        .into_iter()
        .filter(|p| !before.contains(p))
        .collect();
    assert!(
        escaped.is_empty(),
        "a hostile identity wrote OUTSIDE the sink: {escaped:#?}"
    );

    // Paired: a valid identity still produces a file INSIDE the sink, or this
    // is satisfied by a build that writes nothing at all.
    let ok = written_path(&writer(&sink, "ok-1_A").append(&payload("sess", "SessionStart")));
    assert_eq!(ok.parent(), Some(sink.as_path()));
    assert!(ok.is_file());
}

fn list(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = fs::read_dir(dir)
        .map(|rd| rd.filter_map(Result::ok).map(|e| e.path()).collect())
        .unwrap_or_default();
    out.sort();
    out
}

// ---------------------------------------------------------------------------
// §9 (e) — Uniqueness on the normalised filename, not the declared string
// ---------------------------------------------------------------------------

/// ADR-0011 §9 (e). Two identities differing only by case must be refused as
/// duplicates. Paired: two genuinely distinct identities must be accepted.
///
/// **The trailing-dot and trailing-space halves of §9 (e) are refused by the
/// charset, not by the duplicate check**, and that difference is deliberate
/// rather than an oversight — see the module docs on
/// [`identity`](vibe_core::monitor::identity). After
/// [`WriterIdentity::parse`], `ok.` and `ok ` are unrepresentable; the only
/// collision a *valid* pair can still have is case. Both are refusals and
/// neither can produce a twin writer; the reasons differ, and the ADR's wording
/// implies one reason where the implementation has two.
#[test]
fn uniqueness_is_checked_on_the_normalised_filename_not_the_declared_string() {
    let by_case = vec!["alpha".to_owned(), "Alpha".to_owned()];
    for d in &by_case {
        assert!(
            WriterIdentity::parse(d).is_ok(),
            "the fixture must reach the normalisation: {d} must be a VALID \
             identity, or the charset check is what refuses it and this test \
             is about something else"
        );
    }
    let found = collisions(&by_case);
    assert_eq!(
        found.len(),
        1,
        "case-folded identities must collide — measured on Windows 10 Pro \
         19045 to resolve to one file. Comparing raw strings passes cleanly \
         while the filesystem collapses them, which is the twin writer \
         arriving through a check that reads as correct.\n{found:#?}"
    );

    // Paired.
    assert!(collisions(&["alpha".to_owned(), "alpine".to_owned()]).is_empty());

    // The trailing forms, refused one layer earlier. Asserted so the claim in
    // the doc comment above is checked rather than merely written down.
    for stripped in ["ok.", "ok "] {
        assert!(WriterIdentity::parse(stripped).is_err());
        assert_eq!(
            file_key(stripped),
            "ok",
            "the normalisation still folds it, for the config-inspection path"
        );
    }
}

// ---------------------------------------------------------------------------
// §9 (f) — Prunability is derived, paired
// ---------------------------------------------------------------------------

/// ADR-0011 §9 (f). A file containing `SessionEnd` is offered as prunable; a
/// file without one — in progress, or a killed agent — is not, and the two must
/// be reported differently.
///
/// This rests on ADR-0011 §4's measurement rather than on memory: a killed
/// agent writes **neither `Stop` nor `SessionEnd`**, confirmed twice by killing
/// a running agent and reading what was on disk.
#[test]
fn prunability_is_derived_from_session_end() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sink = dir.path();

    let done = writer(sink, "done");
    done.append(&payload("s1", "SessionStart"));
    let done_path = written_path(&done.append(&payload("s1", "SessionEnd")));

    // A killed agent: SessionStart, then a tool, then nothing. Exactly the
    // shape §4 measured.
    let live = writer(sink, "live");
    live.append(&payload("s2", "SessionStart"));
    let live_path = written_path(&live.append(&payload("s2", "PreToolUse")));

    let SinkRead::Read(done_file) = read_file(&done_path) else {
        panic!("readable");
    };
    let SinkRead::Read(live_file) = read_file(&live_path) else {
        panic!("readable");
    };

    assert_eq!(done_file.prunability(), Prunability::Prunable);
    assert_eq!(
        live_file.prunability(),
        Prunability::NotPrunable {
            reason: NotPrunableReason::NoSessionEnd
        },
        "a session with no SessionEnd is in-progress-or-dead and must never be \
         offered as prunable"
    );
    assert_ne!(
        done_file.prunability(),
        live_file.prunability(),
        "the two must be reported differently, or a build offering every file \
         satisfies the first half perfectly"
    );
}

/// A torn tail is its own not-prunable reason. Merging it into
/// `NoSessionEnd` would report *"still running"* about a file we simply do not
/// understand the end of.
#[test]
fn a_torn_tail_is_not_prunable_even_when_session_end_is_present() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("s__ident.jsonl");
    fs::write(
        &path,
        "{\"v\":\"1\",\"session\":\"s\",\"event\":\"SessionEnd\"}\n{\"v\":\"1\",\"ses",
    )
    .expect("fixture");

    let SinkRead::Read(file) = read_file(&path) else {
        panic!("readable");
    };
    assert_eq!(
        file.prunability(),
        Prunability::NotPrunable {
            reason: NotPrunableReason::PartialTail
        }
    );
}

// ---------------------------------------------------------------------------
// The write-failure taxonomy: a hook that cannot write must not be silent
// ---------------------------------------------------------------------------

/// A failed write is **silent non-delivery arriving from our side** — ADR-0011
/// §7's central hazard produced by the mechanism installed to prevent it. It
/// may not be a panic and it may not be a swallow.
///
/// The reachable instance on this machine is a sink path that cannot be
/// created because a **file** sits where the directory must go. That is a real
/// `create_dir_all` failure through the real syscall, not a synthesised value.
#[test]
fn a_sink_that_cannot_be_created_is_reported_rather_than_swallowed_or_panicked() {
    let dir = tempfile::tempdir().expect("tempdir");
    let blocked = dir.path().join("sink");
    fs::write(&blocked, b"I am a file, not a directory").expect("plant the blocker");

    let outcome = writer(&blocked, "ident").append(&payload("s", "SessionStart"));

    match &outcome {
        WriteOutcome::Failed { stage, io, .. } => {
            assert_eq!(*stage, WriteStage::CreateSink);
            println!("  create_dir_all over a file -> {io:?}");
        }
        other => panic!("a blocked sink must be reported, got {other:?}"),
    }
    assert!(
        !outcome.delivered(),
        "a failed write must never report delivery"
    );

    // Paired: the identical call with the blocker removed must succeed, or the
    // assertion above is satisfied by a build that fails unconditionally.
    fs::remove_file(&blocked).expect("unblock");
    assert!(
        writer(&blocked, "ident")
            .append(&payload("s", "SessionStart"))
            .delivered(),
        "with the blocker gone the same write must succeed"
    );
}

/// The `OpenFile` stage, reached deliberately.
///
/// **This control exists because a sabotage found it missing.** Blocking the
/// *sink* with a file makes `create_dir_all` fail, so the write returns at
/// `CreateSink` and never reaches the open — and a sabotage of the open path
/// came back **green** against that fixture. ADR-0002 §7: *a negative control
/// must demonstrate that the sabotaged guard was reached; a test that fails
/// before the guard executes proves nothing about it.* Build the fixture so
/// every step downstream of the check succeeds.
///
/// Here the sink is creatable and a **directory** sits at the record path, so
/// `create_dir_all` succeeds and `OpenOptions::open` is what fails.
#[test]
fn a_record_file_that_cannot_be_opened_is_reported_at_the_open_stage() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sink = dir.path().join("sink");
    let w = writer(&sink, "ident");
    let session = SessionComponent::parse("sess").expect("valid");
    let blocked = w.record_path(&session);
    fs::create_dir_all(&blocked).expect("plant a directory where the record file goes");

    let outcome = w.append(&payload("sess", "SessionStart"));
    match &outcome {
        WriteOutcome::Failed { stage, io, .. } => {
            assert_eq!(
                *stage,
                WriteStage::OpenFile,
                "the sink was creatable, so the failure must be at the open — \
                 if this says CreateSink the fixture is not reaching the guard"
            );
            println!("  open over a directory -> {io:?}");
        }
        other => panic!("an unopenable record file must be reported, got {other:?}"),
    }
    assert!(
        !outcome.delivered(),
        "a failed open must never report delivery"
    );

    // Paired: same writer, same sink, a session whose path is not blocked.
    assert!(
        w.append(&payload("other-sess", "SessionStart")).delivered(),
        "an unblocked path must still succeed, or this is satisfied by a build \
         that fails unconditionally"
    );
}

/// A payload that cannot name a file is refused, and the refusal names which
/// of the five reasons it is. Merging them would report a full disk as a bad
/// payload, and *"no session_id"* as *"not JSON"*.
#[test]
fn each_payload_refusal_is_distinguishable_from_the_others() {
    let dir = tempfile::tempdir().expect("tempdir");
    let w = writer(dir.path(), "ident");

    let cases: [(&str, &str); 5] = [
        ("not json at all", "not_json"),
        ("[1,2,3]", "not_an_object"),
        (r#"{"cwd":"/tmp"}"#, "no_session_id"),
        (r#"{"session_id":7}"#, "session_id_not_string"),
        (r#"{"session_id":"a/b"}"#, "session_rejected"),
    ];

    let mut seen = Vec::new();
    for (payload, expected) in cases {
        match w.append(payload) {
            WriteOutcome::Refused { reason } => {
                assert_eq!(reason.key(), expected, "for payload {payload:?}");
                seen.push(reason.key());
            }
            other => panic!("payload {payload:?} was not refused: {other:?}"),
        }
    }
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen.len(), 5, "two refusals share a key: {seen:?}");

    // Nothing was written for any of them.
    assert!(
        list(dir.path()).is_empty(),
        "a refused payload must leave no file behind: {:?}",
        list(dir.path())
    );
}

// ---------------------------------------------------------------------------
// The authored stamp
// ---------------------------------------------------------------------------

/// The stamp is written when the clock can be read, and **omitted** when it
/// cannot — never zeroed. A zero stamp is a plausible value in the right
/// shape, which is the failure class ADR-0002 §7 records for .NET's
/// `Process.StartTime`: a wrapper that degrades to a well-formed value is worse
/// than one that throws, because the reading it produces is not obviously a
/// reading at all.
#[test]
fn an_unreadable_clock_omits_the_stamp_rather_than_writing_zero() {
    #[derive(Debug)]
    struct DeadClock;
    impl StampSource for DeadClock {
        fn now(&self) -> Option<vibe_core::monitor::Stamp> {
            None
        }
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let w = Writer::new(
        dir.path(),
        WriterIdentity::parse("ident").expect("valid"),
        Arc::new(DeadClock),
    );
    let outcome = w.append(&payload("s", "SessionStart"));
    let path = written_path(&outcome);
    assert!(
        matches!(outcome, WriteOutcome::Written { stamped: false, .. }),
        "the outcome must say the record is unstamped"
    );

    let text = fs::read_to_string(&path).expect("read");
    assert!(
        !text.contains("stamp_ns"),
        "an unreadable clock must omit the field, not write a placeholder: {text}"
    );
    assert!(
        !text.contains(":0"),
        "and must not write a zero anywhere it could be read as a time: {text}"
    );

    // Paired: a readable clock DOES write one, or the assertion above is
    // satisfied by a build that never stamps anything.
    let w2 = Writer::new(
        dir.path(),
        WriterIdentity::parse("ident2").expect("valid"),
        Arc::new(SystemStamps),
    );
    let p2 = written_path(&w2.append(&payload("s", "SessionStart")));
    let t2 = fs::read_to_string(&p2).expect("read");
    assert!(t2.contains("stamp_ns"), "a live clock must stamp: {t2}");
}

/// The stamp is a **string of decimal digits**, not a JSON number.
///
/// Nanoseconds since the epoch is past 2^53, so a JSON number that large loses
/// precision in any reader backed by an IEEE double — every JavaScript one and
/// several `jq` builds. A stamp that quietly changes value on the way out is an
/// instrument altering its own measurement.
#[test]
fn the_stamp_survives_a_round_trip_through_a_double_backed_reader() {
    let dir = tempfile::tempdir().expect("tempdir");
    let w = Writer::new(
        dir.path(),
        WriterIdentity::parse("ident").expect("valid"),
        Arc::new(SystemStamps),
    );
    let path = written_path(&w.append(&payload("s", "SessionStart")));
    let text = fs::read_to_string(&path).expect("read");

    let value: serde_json::Value = serde_json::from_str(text.trim_end()).expect("one JSON line");
    let stamp = value
        .get("stamp_ns")
        .and_then(serde_json::Value::as_str)
        .expect("stamp_ns is a STRING");

    let parsed: u128 = stamp.parse().expect("decimal digits");
    assert_eq!(parsed.to_string(), stamp, "the digits round-trip exactly");

    // The premise, asserted rather than assumed: this value really is past the
    // range a double represents exactly. If it were not, this test would be
    // green for a reason that has nothing to do with what it claims.
    assert!(
        parsed > (1u128 << 53),
        "the fixture's premise failed: {parsed} is inside the exactly-\
         representable range, so nothing here is being tested"
    );
    #[expect(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the precision loss IS the measurement"
    )]
    let through_double = (parsed as f64) as u128;
    assert_ne!(
        through_double, parsed,
        "a double really does lose this value, which is why it is a string"
    );
}

// ---------------------------------------------------------------------------
// The record carries the identity as received
// ---------------------------------------------------------------------------

/// Install and write are two instruments answering one question, and the
/// channel between them can alter the value in transit — ADR-0002 §7's
/// six-for-six population, every instance of which was a channel and not a
/// subject.
///
/// So the record carries the identity **as the writer received it**, the same
/// property `probe.js` gets from echoing argv: a disagreement becomes visible
/// in the artifact rather than being inferred from a missing file.
#[test]
fn the_record_carries_the_identity_the_writer_received() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = written_path(&writer(dir.path(), "alpha").append(&payload("s", "SessionStart")));
    let text = fs::read_to_string(&path).expect("read");
    let value: serde_json::Value = serde_json::from_str(text.trim_end()).expect("json");
    assert_eq!(
        value.get("identity").and_then(|v| v.as_str()),
        Some("alpha")
    );
}

/// The payload lands **byte-identical**, including key order.
///
/// Round-tripping it through `serde_json::Value` would sort the keys — the
/// default map is a `BTreeMap` — and normalise number formatting. That is an
/// instrument altering the subject's data inside a tool whose whole product is
/// reported facts, and ADR-0011 §7a's falsification table expects a malformed
/// payload to *"land in the file exactly as it lands at the receiver"*.
#[test]
fn the_payload_lands_byte_identical_including_key_order() {
    let dir = tempfile::tempdir().expect("tempdir");
    // `zzz` before `aaa`, and a float that a re-serialisation would rewrite.
    let original = r#"{"zzz":1.50,"session_id":"s","aaa":[1,2],"hook_event_name":"Stop"}"#;
    let path = written_path(&writer(dir.path(), "ident").append(original));
    let text = fs::read_to_string(&path).expect("read");
    let value: serde_json::Value = serde_json::from_str(text.trim_end()).expect("json");

    assert_eq!(
        value.get("payload").and_then(|v| v.as_str()),
        Some(original),
        "the payload must be stored verbatim"
    );

    // The premise: a naive round-trip really would have changed it.
    let round_tripped =
        serde_json::to_string(&serde_json::from_str::<serde_json::Value>(original).unwrap())
            .unwrap();
    assert_ne!(
        round_tripped, original,
        "the fixture's premise failed: a round trip did not alter this payload, \
         so it cannot demonstrate that verbatim storage matters"
    );
}

// ---------------------------------------------------------------------------
// Sanity on the fixture helpers themselves
// ---------------------------------------------------------------------------

/// `SessionComponent` and `WriterIdentity` share one charset check, so a change
/// to one cannot silently loosen the other.
#[test]
fn the_two_path_components_share_one_charset() {
    for hostile in ["../x", "a/b", "a:b", ""] {
        assert!(WriterIdentity::parse(hostile).is_err());
        assert!(SessionComponent::parse(hostile).is_err());
    }
    assert!(WriterIdentity::parse("ok-1_A").is_ok());
    assert!(SessionComponent::parse("ok-1_A").is_ok());
}

/// The bounds differ, and the difference is path arithmetic rather than taste.
#[test]
fn the_length_bounds_are_what_the_module_says_they_are() {
    let long_identity = "a".repeat(vibe_core::monitor::IDENTITY_MAX_LEN + 1);
    assert!(matches!(
        WriterIdentity::parse(&long_identity),
        Err(ComponentRejection::TooLong { .. })
    ));
    assert!(WriterIdentity::parse(&"a".repeat(vibe_core::monitor::IDENTITY_MAX_LEN)).is_ok());

    let long_session = "a".repeat(vibe_core::monitor::SESSION_MAX_LEN + 1);
    assert!(matches!(
        SessionComponent::parse(&long_session),
        Err(ComponentRejection::TooLong { .. })
    ));

    // The arithmetic the constants exist for: the longest filename this design
    // can produce must leave room for a sink directory under Windows MAX_PATH.
    let longest = vibe_core::monitor::SESSION_MAX_LEN
        + 2
        + vibe_core::monitor::IDENTITY_MAX_LEN
        + ".jsonl".len();
    assert!(
        longest <= 120,
        "the longest filename is {longest} characters, which spends the \
         MAX_PATH headroom the constants were chosen to preserve"
    );
}

/// Every rejection reason is reachable and distinguishable.
#[test]
fn every_component_rejection_is_reachable() {
    let mut keys = vec![
        WriterIdentity::parse("").unwrap_err().key(),
        WriterIdentity::parse(&"a".repeat(999)).unwrap_err().key(),
        WriterIdentity::parse("a/b").unwrap_err().key(),
    ];
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), 3, "two rejections share a key: {keys:?}");
}

/// `ReadRecord::Unparseable` is a distinct outcome from a partial tail: it has
/// its newline, so it was written whole and is wrong for another reason.
/// Collapsing them would report a hand edit as a crash.
#[test]
fn a_whole_line_that_does_not_parse_is_distinct_from_a_torn_tail() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("s__ident.jsonl");
    fs::write(&path, "this is not json\n").expect("fixture");

    let SinkRead::Read(file) = read_file(&path) else {
        panic!("readable");
    };
    assert_eq!(file.tail, TailState::Complete, "it has its newline");
    assert!(matches!(
        file.records.as_slice(),
        [ReadRecord::Unparseable { .. }]
    ));
}
