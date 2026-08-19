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
    AgentComponent, ComponentRejection, FixedStamps, PayloadRefusal, ReadRecord, SessionComponent,
    SinkRead, StampSource, SystemStamps, TailState, WriteOutcome, WriteStage, Writer,
    WriterIdentity, collisions, file_key, read_file,
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

/// A payload from a **subagent**: same session as its parent, plus `agent_id`.
///
/// Measured on Claude Code 2.1.233 — a subagent shares its parent's
/// `session_id`, and every subagent-owned tool event carries `agent_id` while
/// parent-level events carry none.
fn agent_payload(session: &str, agent: &str, event: &str) -> String {
    format!(
        r#"{{"session_id":"{session}","agent_id":"{agent}","agent_type":"general-purpose","hook_event_name":"{event}","cwd":"/tmp/p"}}"#
    )
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

/// **The fourth axis, added 2026-08-18 after the third key broke.**
///
/// The series this fixture belongs to is: one session, one hook per source, one
/// agent. Every one of them was silent on multiplicity and every discovery came
/// afterwards — ADR-0002 §7's *a fixture with N=1 cannot tell "one" from
/// "exactly one"*. So the axis that broke the two-part key gets its own control
/// with more than one of the thing on the right-hand side of the `per`.
///
/// Measured on Claude Code 2.1.233: **a subagent shares its parent's
/// `session_id`**, and three subagents in parallel still produced exactly one.
/// Twelve pairs of hook processes with the same declared identity were observed
/// alive at the same time. So `(session, identity)` is not one writer.
#[test]
fn one_session_and_one_identity_still_produce_a_file_per_agent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sink = dir.path();

    // One session, one identity — everything the old key had — and three
    // agents plus the parent.
    let w = writer(sink, "alpha");
    let parent = written_path(&w.append(&payload("sess-1", "PreToolUse")));
    let a = written_path(&w.append(&agent_payload("sess-1", "ab8b50189992e6091", "PreToolUse")));
    let b = written_path(&w.append(&agent_payload("sess-1", "a3c4cc9aad124c8e7", "PreToolUse")));
    let c = written_path(&w.append(&agent_payload("sess-1", "a95af8cdec7a03af3", "PreToolUse")));

    let all = [&parent, &a, &b, &c];
    for p in all {
        assert!(p.is_file(), "no file at {}", p.display());
    }
    let mut uniq: Vec<&PathBuf> = all.to_vec();
    uniq.sort();
    uniq.dedup();
    assert_eq!(
        uniq.len(),
        4,
        "the parent and three subagents shared a file under one session and one \
         identity — which is the state measured on 2.1.233 and the reason the \
         key grew a third component.\n{all:#?}"
    );

    // The parent is distinguished by the COMPONENT COUNT, not by a reserved
    // word. Asserted directly, because a reserved literal is the obvious
    // alternative and it could collide with a real agent_id.
    let name = |p: &PathBuf| p.file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!(
        name(&parent).matches("__").count(),
        1,
        "a parent-level record is two components: {}",
        name(&parent)
    );
    assert_eq!(
        name(&a).matches("__").count(),
        2,
        "an agent-level record is three components: {}",
        name(&a)
    );
}

/// The separator rule is what makes the component count readable, so it is a
/// guard rather than a style choice.
///
/// If `__` were legal inside an identity, `s__a__b.jsonl` would be ambiguous
/// between session `s` / agent `a` / identity `b` and session `s` / identity
/// `a__b` — and the parent of one agent would be indistinguishable from a
/// subagent of another.
#[test]
fn no_component_can_form_the_separator_because_underscore_is_not_in_the_charset() {
    // A literal `__` inside a component — what the old check caught.
    for hostile in ["a__b", "__lead", "trail__", "a__b__c"] {
        assert!(
            matches!(
                WriterIdentity::parse(hostile),
                Err(ComponentRejection::IllegalByte { .. })
            ),
            "{hostile:?} was accepted, so the component count is ambiguous"
        );
    }

    // **The forms the old check MISSED**, and the reason for the change: a
    // single `_` at a boundary lets two distinct triples render one filename.
    // `("sess", "abc_", "user")` and `("sess", "abc", "_user")` both produced
    // `sess__abc___user.jsonl`, and both were accepted (ADR-0011 §2 round 3h).
    for boundary in ["a_b", "_lead", "trail_"] {
        assert!(
            matches!(
                WriterIdentity::parse(boundary),
                Err(ComponentRejection::IllegalByte { .. })
            ),
            "{boundary:?} was accepted — a single `_` at a boundary is exactly \
             how the separator got formed from two legal components"
        );
    }

    // Paired, or this has quietly banned everything: the charset still admits
    // what it is meant to.
    assert!(WriterIdentity::parse("a-b").is_ok());
    assert!(WriterIdentity::parse("user").is_ok());
    assert!(WriterIdentity::parse("Alpha9").is_ok());

    // And the point of it all: the two triples that used to collide cannot
    // both be built any more, because one of their components is refused.
    assert!(AgentComponent::parse("abc_").is_err());
    assert!(WriterIdentity::parse("_user").is_err());
}

/// `agent_id` reaches a filename, so it takes the same validation as the
/// identity — and the same paired control.
///
/// ADR-0011 §7a: the `session_id` gap doubles rather than staying one. Observed
/// values are 17 lowercase alphanumerics, which the charset accepts, but that
/// is a **sample of seven** and not a guarantee about the field.
#[test]
fn a_hostile_agent_id_is_refused_rather_than_reaching_a_filename() {
    let dir = tempfile::tempdir().expect("tempdir");
    let w = writer(dir.path(), "ident");

    for hostile in ["../escape", "x/../../escape", "a/b", "a:b", "a__b", ""] {
        let outcome = w.append(&agent_payload("sess", hostile, "PreToolUse"));
        assert!(
            matches!(
                outcome,
                WriteOutcome::Refused {
                    reason: PayloadRefusal::AgentRejected { .. }
                }
            ),
            "agent_id {hostile:?} was not refused: {outcome:?}"
        );
    }

    // Paired: a real observed value is accepted and produces a file.
    assert!(
        w.append(&agent_payload("sess", "ab8b50189992e6091", "PreToolUse"))
            .delivered()
    );

    // And a non-string agent_id is its own refusal, not merged into the above.
    let outcome = w.append(r#"{"session_id":"sess","agent_id":7}"#);
    assert!(
        matches!(
            outcome,
            WriteOutcome::Refused {
                reason: PayloadRefusal::AgentIdNotString
            }
        ),
        "{outcome:?}"
    );

    // Absent agent_id is ORDINARY, not an error — it is every parent event.
    assert!(w.append(&payload("sess", "SessionStart")).delivered());
    assert!(
        w.append(r#"{"session_id":"sess","agent_id":null}"#)
            .delivered(),
        "an explicit null agent_id is absence, not a malformed value"
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
    let outcome = writer(dir.path(), "ok-1-A").append(&payload("s", "SessionStart"));
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
/// # And the platform asymmetry inverts, measured on 2026-08-18
///
/// **Linux does not escape.** Measured under WSL2, kernel 6.18.33.2, with two
/// independent instruments agreeing — a shell redirect and `python3`'s
/// `open(2)`: `x/../../escape` returns `ENOENT`, because Linux resolves `..`
/// against **real directories** and `sess__x` does not exist. Windows
/// canonicalises `..` lexically, so it never asks. Also on Linux: `a:b`
/// creates an ordinary *contained* file — no alternate data stream — and
/// `ok.`/`ok ` are preserved as distinct names rather than folded.
///
/// **macOS: measured by the assertion below, green at `89c2137`.** It was
/// recorded here as *"consistent with the same cause, and consistent-with is
/// not measured"* — true when written, because job logs need a credential this
/// project does not have (ADR-0008 §9) and the red was an aggregate. The Unix
/// arm asserts `escaped.is_empty()` rather than staying silent, so the
/// `Test (macos-latest)` runner takes the measurement on every run; it passed,
/// which is the reading. macOS does not resolve these `..` lexically either.
///
/// The upgrade from *consistent-with* to *measured* is recorded rather than
/// swapped in silently, because which of the two a claim rests on is the fact.
///
/// **This is the inversion ADR-0011 §5 predicted.** It reasoned that if Unix
/// could not produce a real *unavailable* through its own route, the platform
/// limit would invert ADR-0010 §10's, where Unix had the reachable fixture and
/// Windows had the synthesised one. Here Windows holds the reachable fixture
/// and Unix does not.
///
/// So the reachability assertion is Windows-only, and on Unix the containment
/// control below guards a hazard that is **unreachable through this filename
/// scheme on that platform** — a declared platform limit, not coverage. That is
/// ADR-0010 §10's shape exactly: *filing a synthesised value as the best
/// available on all three would be calling something unreachable while it is in
/// reach* on the runner where it is real.
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

    // The premise of control (d), on the platform where the hazard is real.
    //
    // Windows-only because the escape is Windows-only, measured rather than
    // assumed — see the doc comment. Gating it on `cfg` is what keeps the
    // Unix runners honest: they record the hazard as unreachable there instead
    // of reporting a green that would read as containment being demonstrated.
    #[cfg(windows)]
    assert!(
        !escaped.is_empty(),
        "no unvalidated identity escaped the sink on Windows, where the escape \
         was measured on 2026-08-18. Either the filename scheme changed or the \
         platform did; the containment control below is now guarding a hazard \
         nothing here demonstrates. That is a result to record, not a green to \
         accept."
    );

    // The Unix half, and it is an assertion rather than a silence.
    //
    // Measured absent on Linux; **unmeasured on macOS**, where no such
    // measurement was available from this machine. Asserting it here makes the
    // macOS runner take that measurement on every run: a red is not a defect in
    // the writer, it is the discovery that macOS resolves `..` the way Windows
    // does, and the doc comment above becomes wrong rather than incomplete.
    #[cfg(not(windows))]
    assert!(
        escaped.is_empty(),
        "an unvalidated identity escaped the sink on a non-Windows platform. \
         This was measured absent on Linux (WSL2 6.18.33.2) and never measured \
         on macOS, so this red is a finding about the platform: `..` is being \
         resolved lexically here, and control (d)'s reachability comment needs \
         rewriting.\n{escaped:#?}"
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
    let ok = written_path(&writer(&sink, "ok-1-A").append(&payload("sess", "SessionStart")));
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
// §9 (f) — RETRACTED: prunability was not derivable from event content
// ---------------------------------------------------------------------------
//
// Two controls stood here: `prunability_is_derived_from_session_end` and
// `a_torn_tail_is_not_prunable_even_when_session_end_is_present`. Both passed
// throughout, and both were pinning a claim the payload does not support.
//
// ADR-0011 §2 round 3e measured a resumed session emitting `SessionStart`
// AFTER `SessionEnd` under one `session_id`, so "contains SessionEnd" does not
// mean "will receive no more records". The proposed repair — a third state,
// ENDED AT LEAST ONCE AND REOPENABLE — was checked before being built and did
// not survive either: `reopenable` is always true, so the predicate never
// varies and the variant would have distinguished nothing.
//
// The type is gone rather than weakened, and the retraction is recorded at the
// module it belonged to rather than only here, because a retraction leaves
// residue at its copies and the copies are what the next reader meets. What
// must NOT be written in its place — file-age prunability, and why — is in
// `monitor::sink`'s own docs.
//
// Nothing replaces these controls. A control asserting that no prunability is
// offered would be asserting the absence of a type, which the compiler already
// does.

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
    let blocked = w.record_path(&session, None);
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

    // All SEVEN variants. The list grew from five when the key gained a third
    // component, and a case list that reads as complete and is not is the
    // failure this file exists against.
    let cases: [(&str, &str); 7] = [
        ("not json at all", "not_json"),
        ("[1,2,3]", "not_an_object"),
        (r#"{"cwd":"/tmp"}"#, "no_session_id"),
        (r#"{"session_id":7}"#, "session_id_not_string"),
        (r#"{"session_id":"a/b"}"#, "session_rejected"),
        (r#"{"session_id":"s","agent_id":7}"#, "agent_id_not_string"),
        (r#"{"session_id":"s","agent_id":"a/b"}"#, "agent_rejected"),
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
    assert_eq!(seen.len(), 7, "two refusals share a key: {seen:?}");

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

    // The loss is demonstrated on a FIXED value, not on the live clock.
    //
    // **This assertion used to read `(parsed as f64) as u128 != parsed` and was
    // flaky.** Epoch nanoseconds are ~2^61 and an `f64` carries 53 mantissa
    // bits, so a value survives the round trip exactly when its low 8 bits are
    // zero. Windows' clock ticks in multiples of 100 ns, so one reading in
    // every 6400 ns is exactly representable and the assertion failed through
    // no defect — measured at 1 failure in 40 local runs, and it turned CI red
    // on a documentation-only commit.
    //
    // That is precisely what ADR-0002 §7 forbids: *a negative control must fire
    // deterministically; one whose firing depends on winning a race can stop
    // proving anything without ever failing.* The rule is usually quoted about
    // a silent green; here the same non-determinism arrived as a random red,
    // which is the same defect wearing the other colour.
    //
    // So the claim is split. Above: the live stamp is in the range where a
    // double cannot represent every value — deterministic. Below: a double
    // demonstrably loses a value of that magnitude — deterministic, because the
    // value is chosen rather than sampled.
    const LOSSY: u128 = 1_755_000_000_000_000_001;
    #[expect(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the precision loss IS the measurement"
    )]
    let through_double = (LOSSY as f64) as u128;
    assert_ne!(
        through_double, LOSSY,
        "a double does not lose a value of the magnitude this stamp has, so \
         there is no reason for the stamp to be a string and this test is \
         asserting something that is not true"
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

// ---------------------------------------------------------------------------
// Verbatim storage — one control per producer, counted again before the
// `preserve_order` feature flipped
// ---------------------------------------------------------------------------

/// # Why this is several controls and used to be one
///
/// The single control these replace asserted verbatim storage against a fixture
/// carrying **two** reasons a round trip would have changed it — key order and
/// number formatting — and its premise assertion could not tell them apart.
/// Enabling `serde_json/preserve_order` removes the first. That control would
/// have **stayed green on the survivor** while its name, its doc comment and
/// its premise all described something it no longer exercised, and nothing
/// would have gone red to say so.
///
/// That is ADR-0002 §7's claim-outliving-its-retraction, arriving through a
/// control rather than through prose. So the branches were counted again
/// **before** the feature flipped. Measured under `preserve_order`, a naive
/// round trip still changes: **number formatting** (`1.50` becomes `1.5`,
/// `1e2` becomes `100.0`), **insignificant whitespace**, and **a duplicate
/// key** — which is the one that loses data rather than shape. It no longer
/// changes key order, a large integer, or an escaped non-ASCII character.
///
/// Key order is therefore deliberately **not** a premise here any more. The
/// control that pins the new state of that question is
/// [`serde_json_in_this_build_preserves_key_order`].
///
/// A payload that is not JSON at all is **not** in this list: the writer refuses
/// it (`PayloadRefusal::NotJson`) because the filename needs `session_id`, so
/// verbatim storage is not what protects that case and a control here would be
/// asserting the wrong mechanism.
fn assert_lands_verbatim(original: &str) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = written_path(&writer(dir.path(), "ident").append(original));
    let text = fs::read_to_string(&path).expect("read");
    let value: serde_json::Value = serde_json::from_str(text.trim_end()).expect("json");
    assert_eq!(
        value.get("payload").and_then(|v| v.as_str()),
        Some(original),
        "the payload must be stored verbatim"
    );
}

/// The premise for one case: a naive round trip really would have changed it.
///
/// Separate from the assertion above so the failure says which half went —
/// *storage is not verbatim* and *this fixture no longer exercises anything*
/// are different problems with different repairs, and the second is the one
/// that arrives silently.
fn assert_a_round_trip_would_have_changed_it(original: &str, producer: &str) {
    let round_tripped = serde_json::from_str::<serde_json::Value>(original)
        .ok()
        .and_then(|v| serde_json::to_string(&v).ok());
    assert_ne!(
        round_tripped.as_deref(),
        Some(original),
        "the fixture's premise failed: a round trip did not alter this payload, \
         so it no longer demonstrates that verbatim storage protects {producer}"
    );
}

/// Producer 1: number formatting.
#[test]
fn the_payload_keeps_a_number_exactly_as_the_agent_wrote_it() {
    let original = r#"{"session_id":"s","hook_event_name":"Stop","a":1.50,"b":1e2}"#;
    assert_lands_verbatim(original);
    assert_a_round_trip_would_have_changed_it(original, "number formatting");
}

/// Producer 2: insignificant whitespace. The bytes that arrived are the fact.
#[test]
fn the_payload_keeps_the_whitespace_it_arrived_with() {
    let original = r#"{ "session_id" : "s" , "hook_event_name" : "Stop" }"#;
    assert_lands_verbatim(original);
    assert_a_round_trip_would_have_changed_it(original, "insignificant whitespace");
}

/// Producer 3: a duplicate key — the one that loses **data** rather than shape.
/// A round trip keeps one of the two and reports nothing.
#[test]
fn the_payload_keeps_both_halves_of_a_duplicate_key() {
    let original = r#"{"session_id":"s","hook_event_name":"Stop","a":1,"a":2}"#;
    assert_lands_verbatim(original);
    assert_a_round_trip_would_have_changed_it(original, "a duplicate key");
}

/// **`serde_json` preserves key order in this build, and that is a dependency
/// feature rather than a property of the crate.**
///
/// Nothing above exercises key order any more, on purpose. But ADR-0011 §7's
/// settings write **depends** on it: a `Cargo.toml` edit dropping
/// `preserve_order` would make vibe re-sort the keys of a file it does not own,
/// and every other control here would stay green while it happened.
///
/// So the guarantee is checked rather than remembered — the `VIBE_REQUIRE_GH`
/// shape (ADR-0002 §7), where the result carries the evidence and no second
/// channel has to be read. The fixture's own premise is asserted too: keys that
/// were already sorted would make preserving and sorting look identical.
#[test]
fn serde_json_in_this_build_preserves_key_order() {
    let original = r#"{"zzz":1,"mmm":2,"aaa":3}"#;
    let written = ["zzz", "mmm", "aaa"];
    let mut sorted = written;
    sorted.sort_unstable();
    assert_ne!(
        sorted, written,
        "the fixture's premise failed: these keys are already in sorted order, \
         so preserving them and sorting them would look the same"
    );

    let value: serde_json::Value = serde_json::from_str(original).expect("json");
    let round_tripped = serde_json::to_string(&value).expect("serialise");
    assert_eq!(
        round_tripped, original,
        "serde_json re-ordered the keys, so `preserve_order` is not enabled for \
         this build — and the settings write would rewrite the key order of a \
         file vibe does not own"
    );
}

// ---------------------------------------------------------------------------
// The structural argument, and the two things it rests on
// ---------------------------------------------------------------------------

/// **`write_all` must not loop**, because that is what makes a torn record
/// unrepresentable rather than merely unobserved.
///
/// ADR-0011 §2 round 3d: `Writer::append` holds a bare `File` and calls
/// `write_all` once. `write_all` loops over `Write::write` until the buffer is
/// consumed, so a kill can only tear a record if that loop ever runs more than
/// once. Measured on `win32-x64` it never did, at any size to 64 MiB — and that
/// measurement was taken on one machine, which §9 declares as a limit for
/// everything else in that round.
///
/// **This one does not need a live agent session, so unlike the rest of that
/// round it mechanizes.** It runs in the ordinary test job, so the claim is
/// carried on all three platforms instead of resting on the one it was found
/// on. If some platform's `write` returns short, that is the finding, and it
/// arrives as a red rather than as a surprise in a record.
///
/// **The ceiling narrowed from the scratchpad measurement and that is written
/// down rather than left to be reconciled.** The on-demand instrument went to
/// **64 MiB**; this stops at **4 MiB**. A CI runner should not write 64 MiB to
/// establish a property that is about the LOOP rather than about the size, and
/// a real record is **327 bytes** — the first row here — so the narrowing costs
/// nothing that install depends on. What it gives up is the far tail, which the
/// scratchpad covers on demand.
#[test]
fn one_write_call_takes_a_whole_record_on_this_platform() {
    use std::io::Write as _;

    let dir = tempfile::tempdir().expect("tempdir");
    // 327 is a real record, measured from the hook; the rest bracket it.
    for bytes in [327usize, 4 * 1024, 64 * 1024, 1024 * 1024, 4 * 1024 * 1024] {
        let path = dir.path().join(format!("w{bytes}.bin"));
        let buf = vec![b'x'; bytes];
        let mut f = fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&path)
            .expect("open");
        let accepted = f.write(&buf).expect("write");
        assert_eq!(
            accepted,
            bytes,
            "one `write` of {bytes} bytes took only {accepted} on {}/{}. \
             `write_all` would then LOOP, which opens the user-space window \
             ADR-0011 §2 round 3d says does not exist — and the structural \
             argument that a killed hook cannot tear a record dies with it.",
            std::env::consts::OS,
            std::env::consts::ARCH,
        );
    }
}

/// **The write path must have no buffered writer**, because two properties rest
/// on that and one commit would take both.
///
/// `File::flush` being a no-op is why `WriteStage::Flush` was deleted; `write`
/// taking the whole buffer is why a torn record is unrepresentable. Both are
/// properties of `std::fs::File`, not of this code. A `BufWriter` introduced
/// for speed would make the flush live *and* turn the append into a loop, in
/// one change, with **every other control here still green**.
///
/// So the dependency is asserted rather than described. This reads the module's
/// own source, which is the technique `control_inventory.rs` already uses for
/// the same reason: the property is structural, nothing observable distinguishes
/// it until it has already cost something, and a paragraph asking the next
/// author not to do it is a rule that gets broken by someone who never read it.
///
/// **Paired**, or a typo in the path would satisfy it forever: the same read
/// must find the `write_all` call it is reasoning about.
#[test]
fn the_write_path_has_no_buffered_writer() {
    let writer_rs = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("monitor")
        .join("writer.rs");
    let src = fs::read_to_string(&writer_rs)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", writer_rs.display()));

    // THE PREMISE, STATED RATHER THAN GESTURED AT. This assertion establishes
    // two things and the second is the one that rots: that the file was FOUND,
    // and that it still CONTAINS THE WRITE PATH this control is guarding. A
    // rename, a move, or the module being split would leave a readable file
    // with no append in it, and "no BufWriter here" would then be true and
    // worthless. Anchored to a line, and to a line that is not a comment,
    // because the prose in this very file names the append it is about.
    let append_lines: Vec<&str> = src
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .filter(|l| l.contains("write_all(line.as_bytes())"))
        .collect();
    assert_eq!(
        append_lines.len(),
        1,
        "the fixture's premise failed: {} has {} non-comment lines performing \
         the append this control guards, expected exactly one. The write path \
         moved or was split, and finding no `BufWriter` in this file no longer \
         establishes anything about it.",
        writer_rs.display(),
        append_lines.len()
    );

    // Anchored to the line as well: every line is checked on its own, and a
    // comment line is not code. `BufWriter` appears in this module only as
    // prose about why it must not appear.
    let offenders: Vec<(usize, &str)> = src
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.trim_start().starts_with("//"))
        .filter(|(_, l)| l.contains("BufWriter"))
        .map(|(i, l)| (i + 1, l.trim()))
        .collect();
    assert!(
        offenders.is_empty(),
        "a buffered writer appeared in the monitor write path at {offenders:?}. \
         That undoes TWO measured properties at once (ADR-0011 §2 round 3d): \
         `flush` stops being a no-op, so the deleted `WriteStage::Flush` is \
         missing rather than unreachable; and `write_all` becomes a loop, which \
         opens the user-space window a killed hook can tear a record in. Both, \
         from one commit, with nothing else going red."
    );
}

/// **A record is on disk when `append` returns**, which is the behavioural half
/// of the control above.
///
/// **This is the real control and the source check is the corroborator**, which
/// is worth saying because the source one reads as the stronger of the two and
/// is the weaker. It asserts the ABSENCE of a substring, so it is satisfied by
/// the file moving, being renamed, or the module being split — hazards its
/// premise assertion narrows but cannot remove. This one catches buffering by
/// its EFFECT, including through a type that is not called `BufWriter` and
/// including a write path that moved somewhere the source check never looks.
///
/// Two instruments for one property, kept in that order of authority.
#[test]
fn a_written_record_is_on_disk_before_append_returns() {
    let dir = tempfile::tempdir().expect("tempdir");
    let outcome = writer(dir.path(), "ident").append(&payload("sess-flush", "SessionStart"));
    let path = written_path(&outcome);

    // Read through a fresh handle, immediately, with nothing dropped in
    // between: the `Writer` still owns whatever it opened.
    let text = fs::read_to_string(&path).expect("the record must already be readable");
    assert!(
        text.ends_with('\n'),
        "`append` returned Written and the file does not end on a record \
         boundary — bytes are sitting in a buffer somewhere, and the deleted \
         flush stage was load-bearing after all"
    );
    assert!(text.contains("sess-flush"));
}

/// The other half of the observer control: this **is** the live writer.
///
/// A no-op unless `VIBE_HALFWRITE_TARGET` is set, which only
/// [`an_observer_can_see_a_partial_write_on_a_live_file`] does — it re-invokes
/// this test binary with that variable and a filter naming this test. Spawning
/// the test binary rather than adding a helper `[[bin]]` keeps the production
/// surface at zero: a flag on the shipped executable that exists only for a
/// test is a thing users can find.
///
/// **The handshake is files, not sleeps.** Half the record, then a `.half`
/// marker, then a wait for `.go`, then the rest. A timed hold would make this
/// control's firing depend on winning a race against a loaded runner, and
/// ADR-0002 §7 rejects exactly that — it can stop proving anything without ever
/// failing.
///
/// The record both halves agree on is [`HALF_WRITE_RECORD`], shared so the
/// observer is not asserting against a length someone counted by hand.
const HALF_WRITE_RECORD: &[u8] = b"{\"v\":\"1\",\"identity\":\"ident\",\"session\":\"live\"}\n";

#[test]
fn half_write_helper() {
    let Ok(target) = std::env::var("VIBE_HALFWRITE_TARGET") else {
        return;
    };
    let target = PathBuf::from(target);
    let whole = b"{\"v\":\"1\",\"identity\":\"ident\",\"session\":\"live\"}\n";
    let (half, rest) = whole.split_at(whole.len() / 2);

    let mut f = fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(&target)
        .expect("open");
    use std::io::Write as _;
    f.write_all(half).expect("half");

    fs::write(target.with_extension("half"), b"1").expect("marker");
    let go = target.with_extension("go");
    // The handle stays open across this wait, which is the whole point.
    for _ in 0..600 {
        if go.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    f.write_all(rest).expect("rest");
}

/// **An observer can see a partial write on a file a live process holds open.**
///
/// This is the control the kill sweep in ADR-0011 §2 round 3c did not have. Its
/// positive control there was a file truncated **on purpose** — a static file,
/// held open by nobody — which established that the classifier recognises a
/// torn file and established nothing about whether the observer can see one
/// being made. A blind observer produces exactly the clean sweep a healthy one
/// does, so a zero measured through it would have belonged to the instrument.
///
/// It was measured on `win32-x64` in a scratchpad. It is here because the
/// subject is an operating system's file-sharing behaviour, it needs no live
/// agent session, and §9 declares single-platform measurement as a limit for
/// everything that does — so this one is carried on all three instead.
///
/// **Paired both ways**: the same observer must read the whole record once the
/// writer has finished, or *"sees half"* is satisfied by an observer that
/// reports half unconditionally.
#[test]
fn an_observer_can_see_a_partial_write_on_a_live_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("live.jsonl");

    let mut child = std::process::Command::new(std::env::current_exe().expect("current exe"))
        .args(["--exact", "half_write_helper", "--test-threads=1"])
        .env("VIBE_HALFWRITE_TARGET", &target)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn the helper");

    let half_marker = target.with_extension("half");
    let mut waited = 0;
    while !half_marker.exists() && waited < 600 {
        std::thread::sleep(std::time::Duration::from_millis(10));
        waited += 1;
    }
    assert!(
        half_marker.exists(),
        "the helper never reported its half write, so nothing below is a \
         measurement of anything"
    );

    let during = fs::read(&target).expect("read while the writer holds the handle");

    fs::write(target.with_extension("go"), b"1").expect("release the writer");
    child.wait().expect("helper exits");
    let after = fs::read(&target).expect("read after");

    assert!(
        during.len() < after.len() && !during.ends_with(b"\n"),
        "the observer did not resolve a live partial state: it read {} bytes \
         while the writer held the handle and {} after. Any 'no torn file' \
         result measured through this observer belongs to the observer rather \
         than to the subject.",
        during.len(),
        after.len()
    );
    assert!(
        after == HALF_WRITE_RECORD,
        "the observer did not read the whole record after the writer exited, \
         so 'sees half' above is not paired"
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
    assert!(WriterIdentity::parse("ok-1-A").is_ok());
    assert!(SessionComponent::parse("ok-1-A").is_ok());
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
    // Three, not four. `ContainsSeparator` was deleted on 2026-08-19: with `_`
    // outside the charset a component cannot contain `__`, so the variant
    // became unreachable — and an unreachable variant is a representable
    // invalid state, which this project deletes rather than filters. Same move
    // as `WriteStage::Flush`.
    let mut keys = vec![
        WriterIdentity::parse("").unwrap_err().key(),
        WriterIdentity::parse(&"a".repeat(999)).unwrap_err().key(),
        WriterIdentity::parse("a/b").unwrap_err().key(),
    ];
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), 3, "two rejections share a key: {keys:?}");

    // And the deleted one is genuinely unreachable rather than merely unused:
    // the byte check fires first on anything that could have reached it.
    assert_eq!(
        WriterIdentity::parse("a__b").unwrap_err().key(),
        "illegal_byte",
        "`__` must now be refused as an illegal byte, not by a separator check"
    );
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
